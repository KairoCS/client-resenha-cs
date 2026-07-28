// gsi.rs — Game State Integration Manager + Local HTTP Server + Security.
//
// 1) Instala gamestate_integration_resenha.cfg UMA vez (fica fixo na pasta do
//    CS2; nunca é removido — só reescrito se o conteúdo divergir).
// 2) Sobe um servidor HTTP em 127.0.0.1:3210 que recebe os POST do CS2.
// 3) Valida o token secreto (auth.token do payload) — nada de fora do CS2
//    configurado por nós é aceito.
// 4) Se NÃO existe partida ativa da plataforma: descarta o evento (nada sai
//    da máquina). Se existe: extrai eventos e envia pro backend com o matchId.
use crate::events;
use crate::state::AppState;
use crate::ws::send_ws;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::Router;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};
use tracing::{error, info, warn};

pub const GSI_FILE: &str = "gamestate_integration_resenha.cfg";

/// Intervalo do STATE_SYNC (estado condensado) durante uma partida ativa.
const SYNC_INTERVAL: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// Instalação do .cfg
// ---------------------------------------------------------------------------

fn render_cfg(token: &str, port: u16) -> String {
    // Formato KeyValues da Valve. O CS2 lê todo gamestate_integration_*.cfg
    // da pasta csgo/cfg ao iniciar.
    format!(
        r#""Resenha Client"
{{
    "uri" "http://127.0.0.1:{port}/"
    "timeout" "5.0"
    "buffer" "0.1"
    "throttle" "0.1"
    "heartbeat" "30.0"
    "auth"
    {{
        "token" "{token}"
    }}
    "data"
    {{
        "provider"            "1"
        "map"                 "1"
        "round"               "1"
        "player_id"           "1"
        "player_state"        "1"
        "player_match_stats"  "1"
        "player_weapons"      "1"
    }}
}}
"#
    )
}

/// Garante o .cfg na pasta do CS2. Idempotente: só escreve se mudou.
pub fn install_cfg(token: &str, port: u16) -> Result<PathBuf, String> {
    let cfg_dir = find_cs2_cfg_dir().ok_or_else(|| {
        "pasta de configuração do CS2 não encontrada (Steam/CS2 instalados?)".to_string()
    })?;
    let path = cfg_dir.join(GSI_FILE);
    let content = render_cfg(token, port);
    let up_to_date = fs::read_to_string(&path)
        .map(|current| current == content)
        .unwrap_or(false);
    if !up_to_date {
        fs::write(&path, &content).map_err(|e| format!("sem permissão pra escrever o .cfg: {e}"))?;
        info!("GSI instalado em {}", path.display());
    }
    Ok(path)
}

/// Steam → libraryfolders.vdf → biblioteca que contém o app 730 (CS2).
fn find_cs2_cfg_dir() -> Option<PathBuf> {
    let steam = steam_path()?;
    let mut libraries = vec![steam.clone()];

    if let Ok(vdf) = fs::read_to_string(steam.join("steamapps").join("libraryfolders.vdf")) {
        for line in vdf.lines() {
            let line = line.trim();
            if !line.starts_with("\"path\"") {
                continue;
            }
            // formato:  "path"		"D:\\SteamLibrary"
            let parts: Vec<&str> = line.split('"').collect();
            if let Some(raw) = parts.get(3) {
                libraries.push(PathBuf::from(raw.replace("\\\\", "\\")));
            }
        }
    }

    for lib in libraries {
        let steamapps = lib.join("steamapps");
        if steamapps.join("appmanifest_730.acf").exists() {
            let cfg = steamapps
                .join("common")
                .join("Counter-Strike Global Offensive")
                .join("game")
                .join("csgo")
                .join("cfg");
            if cfg.exists() {
                return Some(cfg);
            }
        }
    }
    None
}

#[cfg(windows)]
fn steam_path() -> Option<PathBuf> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Software\\Valve\\Steam")
        .ok()?;
    let path: String = key.get_value("SteamPath").ok()?;
    Some(PathBuf::from(path))
}

#[cfg(not(windows))]
fn steam_path() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".steam").join("steam"))
}

// ---------------------------------------------------------------------------
// Servidor HTTP local (127.0.0.1 apenas — nunca exposto pra internet)
// ---------------------------------------------------------------------------

pub async fn run_server(app: AppHandle, port: u16) {
    let router = Router::new()
        .route("/", post(handle_gsi))
        .with_state(app.clone());

    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
        Ok(listener) => listener,
        Err(e) => {
            // Porta ocupada (outro app usando a 3210?) — sem servidor local não
            // existe coleta nenhuma. A UI mostra isso em vez de fingir que está tudo bem.
            error!("não consegui abrir 127.0.0.1:{port}: {e}");
            crate::state::emit_status(&app);
            return;
        }
    };

    info!("servidor GSI ouvindo em 127.0.0.1:{port}");
    app.state::<AppState>()
        .gsi_listening
        .store(true, std::sync::atomic::Ordering::Relaxed);
    crate::state::emit_status(&app);

    if let Err(e) = axum::serve(listener, router).await {
        error!("servidor GSI caiu: {e}");
        app.state::<AppState>()
            .gsi_listening
            .store(false, std::sync::atomic::Ordering::Relaxed);
        crate::state::emit_status(&app);
    }
}

async fn handle_gsi(State(app): State<AppHandle>, body: String) -> StatusCode {
    // Validação de formato: precisa ser JSON.
    let Ok(payload) = serde_json::from_str::<Value>(&body) else {
        return StatusCode::BAD_REQUEST;
    };

    // Validação de autenticação local: token secreto gerado na instalação e
    // gravado no .cfg. Bloqueia qualquer processo local tentando forjar eventos.
    let expected = {
        let state = app.state::<AppState>();
        let cfg = state.config.lock().unwrap();
        cfg.gsi_token.clone()
    };
    let provided = payload
        .pointer("/auth/token")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !constant_time_eq(provided.as_bytes(), expected.as_bytes()) {
        warn!("POST no GSI com token inválido — descartado");
        return StatusCode::UNAUTHORIZED;
    }

    // Existe partida criada pelo site? Não → ignora (Premier, Competitivo,
    // Casual, DM… nada é processado nem enviado).
    let match_id = {
        let state = app.state::<AppState>();
        let id = *state.active_match.lock().unwrap();
        if id.is_none() {
            // Mantém o snapshot limpo pra não vazar diff de partida alheia
            // caso um START_MATCH chegue no meio de outro jogo.
            state.last_snapshot.lock().unwrap().take();
        }
        id
    };
    let Some(match_id) = match_id else {
        return StatusCode::OK;
    };

    // Partida ativa: diff do snapshot anterior → eventos de domínio.
    let (game_events, want_sync) = {
        let state = app.state::<AppState>();
        let mut snapshot = state.last_snapshot.lock().unwrap();
        let evs = events::extract(snapshot.as_ref(), &payload);
        *snapshot = Some(payload.clone());
        drop(snapshot);

        // Sem bloco "map" (jogador no menu, aguardando registro do placar)
        // não há o que sincronizar — evita STATE_SYNC todo-nulo a cada 10s.
        let has_map = payload.get("map").is_some();
        let mut last_sync = state.last_sync.lock().unwrap();
        let want_sync = has_map && last_sync.map_or(true, |t| t.elapsed() >= SYNC_INTERVAL);
        if want_sync {
            *last_sync = Some(Instant::now());
        }
        (evs, want_sync)
    };

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    for ev in game_events {
        send_ws(
            &app,
            json!({
                "type": ev.kind,
                "matchId": match_id,
                "timestamp": ts,
                "data": ev.data,
            }),
        );
    }

    // Estado condensado periódico: mantém o backend consistente mesmo que
    // algum evento se perca numa reconexão.
    if want_sync {
        send_ws(
            &app,
            json!({
                "type": "STATE_SYNC",
                "matchId": match_id,
                "timestamp": ts,
                "data": events::condensed(&payload),
            }),
        );
    }

    StatusCode::OK
}

/// Comparação em tempo constante (evita timing attack local no token).
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
