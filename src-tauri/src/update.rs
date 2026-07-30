// update.rs — checagem de versão. O backend é a fonte de verdade: enquanto
// houver atualização obrigatória pendente, o client não conecta no WebSocket
// e descarta todo evento do CS2 (ver gsi.rs). A UI trava na tela de update.
//
// Duas fontes, de propósito:
//   1. GET {backend}/api/client/version — no boot e de tempos em tempos, pra
//      avisar o usuário mesmo antes de ele logar;
//   2. UPDATE_REQUIRED no HELLO do WebSocket (ws.rs) — o portão de verdade,
//      que vale mesmo se a checagem HTTP tiver falhado.
use crate::state::{emit_status, AppState, UpdateInfo};
use serde::Deserialize;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tracing::{info, warn};

/// Versão deste binário (vem do Cargo.toml).
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// De quanto em quanto tempo reconsultamos o backend (o app fica aberto na
/// bandeja por dias).
const INTERVALO: Duration = Duration::from_secs(2 * 60 * 60);

#[derive(Deserialize)]
struct VersionResponse {
    version: String,
    #[serde(default)]
    url: String,
}

/// a < b em semver simples (x.y.z; partes ausentes valem 0).
pub fn versao_menor(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> [u32; 3] {
        let mut out = [0u32; 3];
        for (i, parte) in v.split('.').take(3).enumerate() {
            out[i] = parte.trim().parse().unwrap_or(0);
        }
        out
    };
    parse(a) < parse(b)
}

/// Marca (ou limpa) a exigência de atualização, derruba o WS e avisa a UI.
pub fn definir(app: &AppHandle, info: Option<UpdateInfo>) {
    let state = app.state::<AppState>();
    let mudou = {
        let mut atual = state.update_required.lock().unwrap();
        let mudou = atual.is_some() != info.is_some();
        *atual = info.clone();
        mudou
    };
    if info.is_some() {
        // Bloqueado: nada de coleta enquanto não atualizar.
        *state.active_match.lock().unwrap() = None;
        *state.last_snapshot.lock().unwrap() = None;
        *state.last_sync.lock().unwrap() = None;
    }
    if mudou {
        // Acorda o WS manager: ele desconecta (ou volta a conectar).
        let _ = state.update_tx.send(info.is_some());
    }
    emit_status(app);
}

/// Consulta o backend uma vez. Falha de rede não bloqueia nada — o portão do
/// HELLO cobre o caso de o client estar velho e a checagem não ter respondido.
pub async fn verificar(app: &AppHandle) {
    let url = format!("{}/api/client/version", crate::config::backend_url());

    let resposta = match reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
    {
        Ok(http) => http.get(&url).send().await,
        Err(e) => {
            warn!("cliente HTTP inválido na checagem de versão: {e}");
            return;
        }
    };

    let dados = match resposta {
        Ok(r) if r.status().is_success() => match r.json::<VersionResponse>().await {
            Ok(d) => d,
            Err(e) => {
                warn!("resposta inválida na checagem de versão: {e}");
                return;
            }
        },
        Ok(r) => {
            // Backend antigo (sem a rota) não exige atualização nenhuma.
            info!("checagem de versão respondeu HTTP {}", r.status());
            return;
        }
        Err(e) => {
            info!("checagem de versão indisponível ({e}) — seguindo normalmente");
            return;
        }
    };

    if versao_menor(CURRENT, &dados.version) {
        info!("atualização obrigatória: v{CURRENT} → v{}", dados.version);
        definir(
            app,
            Some(UpdateInfo {
                latest: dados.version,
                url: dados.url,
            }),
        );
    } else {
        definir(app, None);
    }
}

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            verificar(&app).await;
            tokio::time::sleep(INTERVALO).await;
        }
    });
}
