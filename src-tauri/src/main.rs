// Resenha Client — roda na bandeja, integra o CS2 (GSI) com a plataforma.
//
// Fluxo em uma frase: o CS2 posta o estado do jogo num servidor local, o
// client filtra o que é de partida criada pelo site e manda pro backend por
// WebSocket. Fora de partida da plataforma, nada sai da máquina.
//
//   commands.rs  o que a UI pode pedir       state.rs   estado compartilhado
//   auth.rs      pareamento e tokens          config.rs  config + endereços
//   ws.rs        conexão com o backend        update.rs  versão obrigatória
//   gsi/         integração com o CS2         tray.rs    ícone na bandeja
//
// Sem console em release:
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod auth;
mod commands;
mod config;
mod events;
mod gsi;
mod state;
mod tray;
mod update;
mod ws;

use state::{emit_status, AppState};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use tauri::{Manager, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};
use tracing_subscriber::fmt::writer::MakeWriterExt;

fn main() {
    let _log_guard = init_logging();

    let (ws_tx, ws_rx) = mpsc::unbounded_channel();
    let (session_tx, session_rx) = watch::channel(false);
    let (update_tx, update_rx) = watch::channel(false);

    let app_state = AppState {
        config: Mutex::new(config::load()),
        tokens: auth::TokenManager::new(),
        update_required: Mutex::new(None),
        update_tx,
        active_match: Mutex::new(None),
        ws_connected: AtomicBool::new(false),
        ws_tx,
        session_tx,
        last_snapshot: Mutex::new(None),
        last_sync: Mutex::new(None),
        gsi_path: Mutex::new(None),
        gsi_listening: AtomicBool::new(false),
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
            commands::login,
            commands::logout,
            commands::get_status,
            commands::set_autostart,
            commands::open_logs,
            commands::baixar_atualizacao,
            commands::verificar_atualizacao
        ])
        .setup(move |app| {
            let handle = app.handle().clone();

            tray::setup(&handle)?;
            iniciar_gsi(&handle);

            // WebSocket manager (fica dormindo até existir sessão) e checagem
            // de versão (no boot e a cada 2h).
            ws::spawn(handle.clone(), ws_rx, session_rx, update_rx);
            update::spawn(handle.clone());

            restaurar_sessao(handle.clone());

            info!("Resenha Client v{} iniciado", update::CURRENT);
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

/// Instala o .cfg na pasta do CS2 e sobe o servidor local que recebe o GSI.
fn iniciar_gsi(handle: &tauri::AppHandle) {
    let (token, porta) = {
        let state = handle.state::<AppState>();
        let cfg = state.config.lock().unwrap();
        (cfg.gsi_token.clone(), cfg.gsi_port)
    };

    match gsi::install_cfg(&token, porta) {
        Ok(path) => {
            *handle.state::<AppState>().gsi_path.lock().unwrap() = Some(path.display().to_string());
        }
        Err(e) => warn!("GSI não instalado: {e}"),
    }

    tauri::async_runtime::spawn(gsi::run_server(handle.clone(), porta));
}

/// Decide o que aparece na primeira execução do processo:
/// - sem login → mostra a janela (primeira vez de todas);
/// - com login + "--minimized" (autostart) → fica só na bandeja;
/// - com login, aberto manualmente → mostra o painel.
fn restaurar_sessao(handle: tauri::AppHandle) {
    let minimizado = std::env::args().any(|a| a == "--minimized");

    tauri::async_runtime::spawn(async move {
        let logado = {
            let state = handle.state::<AppState>();
            let logado = state.tokens.load().await.is_some();
            if logado {
                let _ = state.session_tx.send(true);
            }
            logado
        };

        if !logado || !minimizado {
            tray::show_window(&handle);
        }
        emit_status(&handle);
    });
}

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
