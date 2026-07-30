// Resenha Client — roda na bandeja, integra o CS2 (GSI) com a plataforma.
// Sem console em release:
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod auth;
mod config;
mod events;
mod gsi;
mod state;
mod tray;
mod update;
mod ws;

use state::{emit_status, AppState};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};
use tracing_subscriber::fmt::writer::MakeWriterExt;

// ---------------------------------------------------------------------------
// Comandos invocados pela UI
// ---------------------------------------------------------------------------

/// Pareia o client com a conta do site (login lá é via Steam): troca o código
/// de conexão gerado no perfil por access/refresh tokens.
#[tauri::command]
async fn login(app: AppHandle, code: String) -> Result<(), String> {
    let code = code.trim().to_uppercase();
    if code.is_empty() {
        return Err("digite o código de conexão gerado no site".into());
    }
    let tokens = auth::pair(&config::backend_url(), &code)
        .await
        .map_err(|e| e.to_string())?;
    let state = app.state::<AppState>();
    state.tokens.store(tokens).await;
    let _ = state.session_tx.send(true);
    info!("client pareado com a conta do site");
    emit_status(&app);
    Ok(())
}

#[tauri::command]
async fn logout(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    state.tokens.clear().await;
    *state.active_match.lock().unwrap() = None;
    *state.last_snapshot.lock().unwrap() = None;
    let _ = state.session_tx.send(false);
    info!("logout realizado");
    emit_status(&app);
    Ok(())
}

#[tauri::command]
async fn get_status(app: AppHandle) -> state::StatusPayload {
    // Garante que o cache de tokens foi consultado (primeira chamada da UI).
    let state = app.state::<AppState>();
    let _ = state.tokens.load().await;
    state::status_payload(&app)
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let manager = app.autolaunch();
    let result = if enabled { manager.enable() } else { manager.disable() };
    result.map_err(|e| e.to_string())?;
    emit_status(&app);
    Ok(())
}

#[tauri::command]
fn open_logs() {
    let dir = config::log_dir();
    let _ = std::fs::create_dir_all(&dir);
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg(&dir).spawn();
    }
}

/// Abre o instalador da versão nova no navegador padrão.
#[tauri::command]
fn baixar_atualizacao(app: AppHandle) -> Result<(), String> {
    let url = {
        let state = app.state::<AppState>();
        let info = state.update_required.lock().unwrap();
        info.as_ref().map(|u| u.url.clone()).unwrap_or_default()
    };
    // Só http(s): o valor vem do backend, mas nunca passamos string arbitrária
    // pro shell do sistema.
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("o servidor não informou um link de download válido".into());
    }
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &url])
            .spawn()
            .map_err(|e| format!("não consegui abrir o navegador: {e}"))?;
    }
    Ok(())
}

/// Reconsulta o backend na hora (botão "já atualizei / verificar de novo").
#[tauri::command]
async fn verificar_atualizacao(app: AppHandle) {
    update::verificar(&app).await;
}

// ---------------------------------------------------------------------------

fn init_logging() -> tracing_appender::non_blocking::WorkerGuard {
    let log_dir = config::log_dir();
    let _ = std::fs::create_dir_all(&log_dir);
    let file_appender = tracing_appender::rolling::daily(&log_dir, "resenha-client.log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(file_writer.and(std::io::stdout))
        .with_ansi(false)
        .init();
    guard
}

fn main() {
    let _log_guard = init_logging();
    let app_config = config::load();

    let (ws_tx, ws_rx) = mpsc::unbounded_channel();
    let (session_tx, session_rx) = watch::channel(false);
    let (update_tx, update_rx) = watch::channel(false);

    let app_state = AppState {
        config: Mutex::new(app_config),
        tokens: auth::TokenManager::new(),
        update_required: Mutex::new(None),
        update_tx,
        active_match: Mutex::new(None),
        ws_connected: std::sync::atomic::AtomicBool::new(false),
        ws_tx,
        session_tx,
        last_snapshot: Mutex::new(None),
        last_sync: Mutex::new(None),
        gsi_path: Mutex::new(None),
        gsi_listening: std::sync::atomic::AtomicBool::new(false),
    };

    tauri::Builder::default()
        // Instância única: abrir de novo só foca a janela existente.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            tray::show_window(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            login,
            logout,
            get_status,
            set_autostart,
            open_logs,
            baixar_atualizacao,
            verificar_atualizacao
        ])
        .setup(move |app| {
            let handle = app.handle().clone();

            tray::setup(&handle)?;

            // GSI: garante o .cfg fixo na pasta do CS2 e sobe o servidor local.
            let (gsi_token, gsi_port) = {
                let state = handle.state::<AppState>();
                let cfg = state.config.lock().unwrap();
                (cfg.gsi_token.clone(), cfg.gsi_port)
            };
            match gsi::install_cfg(&gsi_token, gsi_port) {
                Ok(path) => {
                    let state = handle.state::<AppState>();
                    *state.gsi_path.lock().unwrap() = Some(path.display().to_string());
                }
                Err(e) => warn!("GSI não instalado: {e}"),
            }
            tauri::async_runtime::spawn(gsi::run_server(handle.clone(), gsi_port));

            // WebSocket manager (fica dormindo até existir sessão).
            ws::spawn(handle.clone(), ws_rx, session_rx, update_rx);

            // Checagem de versão: no boot e a cada 2h.
            update::spawn(handle.clone());

            // Sessão + visibilidade da janela:
            // - sem login → mostra a janela (primeira execução);
            // - com login + "--minimized" (autostart) → fica só na bandeja;
            // - com login, aberto manualmente → mostra o painel.
            let started_minimized = std::env::args().any(|a| a == "--minimized");
            let handle2 = handle.clone();
            tauri::async_runtime::spawn(async move {
                let logged_in = {
                    let state = handle2.state::<AppState>();
                    let logged_in = state.tokens.load().await.is_some();
                    if logged_in {
                        let _ = state.session_tx.send(true);
                    }
                    logged_in
                };
                if !logged_in || !started_minimized {
                    tray::show_window(&handle2);
                }
                emit_status(&handle2);
            });

            info!(
                "Resenha Client v{} iniciado (minimized={})",
                env!("CARGO_PKG_VERSION"),
                started_minimized
            );
            Ok(())
        })
        // Fechar a janela = esconder pra bandeja (o app continua rodando).
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .run(tauri::generate_context!())
        .expect("falha ao iniciar o Resenha Client");
}
