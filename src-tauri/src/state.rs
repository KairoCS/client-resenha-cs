// state.rs — estado global do app (Match Controller + status pra UI/tray).
use crate::auth::TokenManager;
use crate::config::AppConfig;
use serde::Serialize;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;
use tokio::sync::{mpsc, watch};

pub struct AppState {
    pub config: Mutex<AppConfig>,
    pub tokens: TokenManager,

    /// Match Controller: id da partida criada pelo site (None = modo de espera,
    /// todo evento GSI é descartado sem sair da máquina).
    pub active_match: Mutex<Option<i64>>,

    pub ws_connected: AtomicBool,
    /// Fila de mensagens de saída pro WebSocket (GSI → backend).
    pub ws_tx: mpsc::UnboundedSender<Value>,
    /// true = existe sessão (tokens); false = deslogado. O WS manager observa.
    pub session_tx: watch::Sender<bool>,

    /// Último payload GSI (pra diff → eventos). Zerado a cada START/END_MATCH.
    pub last_snapshot: Mutex<Option<Value>>,
    /// Controle do STATE_SYNC periódico durante a partida.
    pub last_sync: Mutex<Option<Instant>>,

    /// Caminho do gamestate_integration_resenha.cfg instalado (None = CS2 não achado).
    pub gsi_path: Mutex<Option<String>>,
    /// Servidor HTTP local no ar? (false = porta ocupada → sem coleta possível)
    pub gsi_listening: AtomicBool,
}

#[derive(Serialize, Clone)]
pub struct StatusPayload {
    pub logged_in: bool,
    pub ws_connected: bool,
    pub active_match: Option<i64>,
    pub gsi_path: Option<String>,
    pub gsi_listening: bool,
    pub gsi_port: u16,
    pub backend_url: String,
    pub autostart: bool,
}

pub fn status_payload(app: &AppHandle) -> StatusPayload {
    let state = app.state::<AppState>();
    let (backend_url, gsi_port) = {
        let cfg = state.config.lock().unwrap();
        (cfg.backend_url.clone(), cfg.gsi_port)
    };
    let payload = StatusPayload {
        logged_in: state.tokens.is_logged_in(),
        ws_connected: state.ws_connected.load(Ordering::Relaxed),
        active_match: *state.active_match.lock().unwrap(),
        gsi_path: state.gsi_path.lock().unwrap().clone(),
        gsi_listening: state.gsi_listening.load(Ordering::Relaxed),
        gsi_port,
        backend_url,
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
    };
    payload
}

/// Notifica a UI (e quem mais escutar) que o status mudou.
pub fn emit_status(app: &AppHandle) {
    let _ = app.emit("status", status_payload(app));
}
