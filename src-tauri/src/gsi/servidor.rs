// gsi/servidor.rs — servidor HTTP local que recebe os POST do CS2.
//
// Escuta SOMENTE em 127.0.0.1 (nunca exposto à rede) e aplica, nesta ordem,
// os filtros que sustentam a promessa da plataforma:
//   1. token secreto  — só o CS2 configurado por nós é aceito;
//   2. versão do app  — client desatualizado não coleta;
//   3. partida ativa  — sem partida criada pelo site, o evento é DESCARTADO
//      (Premier, Competitivo, Casual e DM nunca são monitorados).
use crate::events;
use crate::state::AppState;
use crate::ws::send_ws;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::Router;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};
use tracing::{error, info, warn};

/// Intervalo do STATE_SYNC (estado condensado) durante uma partida ativa.
const SYNC_INTERVAL: Duration = Duration::from_secs(10);

pub async fn run_server(app: AppHandle, port: u16) {
    let router = Router::new().route("/", post(handle_gsi)).with_state(app.clone());

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
    app.state::<AppState>().gsi_listening.store(true, Ordering::Relaxed);
    crate::state::emit_status(&app);

    if let Err(e) = axum::serve(listener, router).await {
        error!("servidor GSI caiu: {e}");
        app.state::<AppState>().gsi_listening.store(false, Ordering::Relaxed);
        crate::state::emit_status(&app);
    }
}

async fn handle_gsi(State(app): State<AppHandle>, body: String) -> StatusCode {
    let Ok(payload) = serde_json::from_str::<Value>(&body) else {
        return StatusCode::BAD_REQUEST;
    };

    if !token_confere(&app, &payload) {
        warn!("POST no GSI com token inválido — descartado");
        return StatusCode::UNAUTHORIZED;
    }

    // Atualização obrigatória pendente → client bloqueado: nada é processado
    // nem sai da máquina até o usuário instalar a versão nova (ver update.rs).
    if app.state::<AppState>().update_required.lock().unwrap().is_some() {
        return StatusCode::OK;
    }

    let Some(match_id) = partida_ativa(&app) else {
        return StatusCode::OK;
    };

    let (eventos, quer_sync) = extrair(&app, &payload);
    let ts = agora_ms();

    for ev in eventos {
        send_ws(&app, json!({
            "type": ev.kind,
            "matchId": match_id,
            "timestamp": ts,
            "data": ev.data,
        }));
    }

    // Estado condensado periódico: mantém o backend consistente mesmo que
    // algum evento se perca numa reconexão.
    if quer_sync {
        send_ws(&app, json!({
            "type": "STATE_SYNC",
            "matchId": match_id,
            "timestamp": ts,
            "data": events::condensed(&payload),
        }));
    }

    StatusCode::OK
}

/// Token secreto gerado na instalação e gravado no .cfg. Bloqueia qualquer
/// processo local tentando forjar eventos.
fn token_confere(app: &AppHandle, payload: &Value) -> bool {
    let esperado = {
        let state = app.state::<AppState>();
        let cfg = state.config.lock().unwrap();
        cfg.gsi_token.clone()
    };
    let recebido = payload.pointer("/auth/token").and_then(Value::as_str).unwrap_or("");
    constant_time_eq(recebido.as_bytes(), esperado.as_bytes())
}

/// Id da partida da plataforma, ou None (e aí nada é processado).
fn partida_ativa(app: &AppHandle) -> Option<i64> {
    let state = app.state::<AppState>();
    let id = *state.active_match.lock().unwrap();
    if id.is_none() {
        // Mantém o snapshot limpo pra não vazar diff de partida alheia
        // caso um START_MATCH chegue no meio de outro jogo.
        state.last_snapshot.lock().unwrap().take();
    }
    id
}

/// Diff do snapshot anterior → eventos de domínio, e se é hora do STATE_SYNC.
fn extrair(app: &AppHandle, payload: &Value) -> (Vec<events::GameEvent>, bool) {
    let state = app.state::<AppState>();

    let mut snapshot = state.last_snapshot.lock().unwrap();
    let eventos = events::extract(snapshot.as_ref(), payload);
    *snapshot = Some(payload.clone());
    drop(snapshot);

    // Sem bloco "map" (jogador no menu, aguardando registro do placar)
    // não há o que sincronizar — evita STATE_SYNC todo-nulo a cada 10s.
    let tem_mapa = payload.get("map").is_some();
    let mut last_sync = state.last_sync.lock().unwrap();
    let quer_sync = tem_mapa && last_sync.map_or(true, |t| t.elapsed() >= SYNC_INTERVAL);
    if quer_sync {
        *last_sync = Some(Instant::now());
    }

    (eventos, quer_sync)
}

fn agora_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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
