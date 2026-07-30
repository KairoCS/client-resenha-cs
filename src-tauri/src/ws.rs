// ws.rs — WebSocket Manager: conexão persistente com o backend dedicado.
// Reconecta sozinho (backoff exponencial com teto de 60s), renova o access
// token quando o handshake devolve 401 e processa START_MATCH / END_MATCH.
use crate::auth::{self, AuthError};
use crate::state::{emit_status, AppState};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tracing::{info, warn};

/// Enfileira uma mensagem pro backend (o WS manager envia quando conectado).
pub fn send_ws(app: &AppHandle, value: Value) {
    let state = app.state::<AppState>();
    let _ = state.ws_tx.send(value);
}

pub fn spawn(
    app: AppHandle,
    mut out_rx: mpsc::UnboundedReceiver<Value>,
    mut session_rx: watch::Receiver<bool>,
    mut update_rx: watch::Receiver<bool>,
) {
    tauri::async_runtime::spawn(async move {
        let mut backoff: u64 = 1;
        loop {
            // Espera existir uma sessão (login feito).
            if !*session_rx.borrow() {
                if session_rx.changed().await.is_err() {
                    return; // app encerrando
                }
                backoff = 1;
                continue;
            }

            // Atualização obrigatória pendente: não adianta reconectar, o
            // backend recusa no HELLO. Espera a flag mudar (o usuário
            // atualiza e reinicia, ou o backend volta atrás na exigência).
            if *update_rx.borrow() {
                if update_rx.changed().await.is_err() {
                    return;
                }
                backoff = 1;
                continue;
            }

            let result =
                connect_and_run(&app, &mut out_rx, &mut session_rx, &mut update_rx, &mut backoff)
                    .await;

            {
                let state = app.state::<AppState>();
                state.ws_connected.store(false, Ordering::Relaxed);
            }
            emit_status(&app);

            match result {
                Ok(()) => info!("WS desconectado, reconectando em {backoff}s"),
                Err(e) => warn!("WS falhou ({e}), nova tentativa em {backoff}s"),
            }

            // Espera o backoff, mas acorda na hora se a sessão mudar (logout/login).
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(backoff)) => {}
                _ = session_rx.changed() => {}
            }
            backoff = (backoff * 2).min(60);
        }
    });
}

async fn connect_and_run(
    app: &AppHandle,
    out_rx: &mut mpsc::UnboundedReceiver<Value>,
    session_rx: &mut watch::Receiver<bool>,
    update_rx: &mut watch::Receiver<bool>,
    backoff: &mut u64,
) -> Result<(), String> {
    let backend_url = crate::config::backend_url();
    let ws_url = crate::config::ws_url();
    let tokens = app
        .state::<AppState>()
        .tokens
        .load()
        .await
        .ok_or_else(|| "sem tokens salvos".to_string())?;

    // 1ª tentativa com o access token atual; se o handshake responder 401,
    // renova com o refresh token e tenta mais uma vez.
    let ws = match try_connect(&ws_url, &tokens.access).await {
        Ok(ws) => ws,
        Err(WsError::Http(resp)) if resp.status().as_u16() == 401 => {
            info!("access token expirado, renovando sessão");
            match auth::refresh(&backend_url, &tokens.refresh).await {
                Ok(new_tokens) => {
                    let state = app.state::<AppState>();
                    state.tokens.store(new_tokens.clone()).await;
                    try_connect(&ws_url, &new_tokens.access)
                        .await
                        .map_err(|e| e.to_string())?
                }
                Err(AuthError::Unauthorized) => {
                    // Refresh token morto: sessão acabou de verdade → volta pro login.
                    warn!("refresh token rejeitado — sessão expirada");
                    let state = app.state::<AppState>();
                    state.tokens.clear().await;
                    *state.active_match.lock().unwrap() = None;
                    let _ = state.session_tx.send(false);
                    let _ = app.emit("session-expired", ());
                    emit_status(app);
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                    return Err("sessão expirada".into());
                }
                Err(AuthError::Other(e)) => return Err(e),
            }
        }
        Err(e) => return Err(e.to_string()),
    };

    // Conectou: zera o backoff e avisa a UI.
    *backoff = 1;
    {
        let state = app.state::<AppState>();
        state.ws_connected.store(true, Ordering::Relaxed);
    }
    emit_status(app);
    info!("WS conectado em {ws_url}");

    let (mut sink, mut stream) = ws.split();

    let hello = json!({
        "type": "HELLO",
        "client": "resenha-client",
        "version": env!("CARGO_PKG_VERSION"),
    });
    let _ = sink.send(Message::Text(hello.to_string())).await;

    let mut ping = tokio::time::interval(Duration::from_secs(30));
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ping.tick().await; // o primeiro tick dispara na hora — descarta

    loop {
        tokio::select! {
            msg = stream.next() => match msg {
                Some(Ok(Message::Text(txt))) => handle_server_message(app, &txt),
                Some(Ok(Message::Ping(data))) => { let _ = sink.send(Message::Pong(data)).await; }
                Some(Ok(Message::Close(_))) | None => return Ok(()),
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(e.to_string()),
            },
            out = out_rx.recv() => match out {
                Some(v) => sink
                    .send(Message::Text(v.to_string()))
                    .await
                    .map_err(|e| e.to_string())?,
                None => return Ok(()),
            },
            _ = ping.tick() => sink
                .send(Message::Ping(Vec::new()))
                .await
                .map_err(|e| e.to_string())?,
            _ = session_rx.changed() => {
                if !*session_rx.borrow() {
                    // Logout: fecha educadamente e volta a esperar sessão.
                    let _ = sink.send(Message::Close(None)).await;
                    return Ok(());
                }
            }
            _ = update_rx.changed() => {
                if *update_rx.borrow() {
                    // Virou obrigatória a atualização: sai de cena.
                    let _ = sink.send(Message::Close(None)).await;
                    return Ok(());
                }
            }
        }
    }
}

async fn try_connect(
    ws_url: &str,
    access_token: &str,
) -> Result<
    tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    WsError,
> {
    let mut request = ws_url.into_client_request()?;
    let header = HeaderValue::from_str(&format!("Bearer {access_token}"))
        .map_err(|e| WsError::Url(tokio_tungstenite::tungstenite::error::UrlError::UnableToConnect(e.to_string())))?;
    request.headers_mut().insert("Authorization", header);
    let (ws, _resp) = tokio_tungstenite::connect_async(request).await?;
    Ok(ws)
}

/// Comandos do backend: START_MATCH liga a coleta, END_MATCH volta à espera.
fn handle_server_message(app: &AppHandle, txt: &str) {
    let Ok(msg) = serde_json::from_str::<Value>(txt) else {
        warn!("mensagem WS inválida (não é JSON)");
        return;
    };
    match msg.get("type").and_then(Value::as_str) {
        Some("START_MATCH") => {
            let Some(match_id) = msg.get("matchId").and_then(Value::as_i64) else {
                warn!("START_MATCH sem matchId — ignorado");
                return;
            };
            {
                let state = app.state::<AppState>();
                *state.active_match.lock().unwrap() = Some(match_id);
                *state.last_snapshot.lock().unwrap() = None;
                *state.last_sync.lock().unwrap() = None;
            }
            info!("START_MATCH recebido — coletando eventos da partida #{match_id}");
            send_ws(app, json!({ "type": "MATCH_ACK", "matchId": match_id }));
            emit_status(app);
        }
        Some("END_MATCH") => {
            let ended = {
                let state = app.state::<AppState>();
                let ended = state.active_match.lock().unwrap().take();
                *state.last_snapshot.lock().unwrap() = None;
                *state.last_sync.lock().unwrap() = None;
                ended
            };
            info!("END_MATCH recebido — partida {ended:?} encerrada, voltando à espera");
            send_ws(app, json!({ "type": "MATCH_ENDED_ACK" }));
            emit_status(app);
        }
        Some("UPDATE_REQUIRED") => {
            // Portão do backend: esta versão não opera mais. Bloqueia tudo e
            // manda a UI pra tela de atualização.
            let latest = msg
                .get("latest")
                .and_then(Value::as_str)
                .unwrap_or("mais recente")
                .to_string();
            let url = msg.get("url").and_then(Value::as_str).unwrap_or("").to_string();
            warn!("backend exige atualização (v{} disponível) — coleta bloqueada", latest);
            crate::update::definir(app, Some(crate::state::UpdateInfo { latest, url }));
        }
        Some("PING") => send_ws(app, json!({ "type": "PONG" })),
        other => info!("mensagem WS não tratada: {other:?}"),
    }
}
