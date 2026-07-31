// commands.rs — a superfície que a UI (main.ts) enxerga. Cada comando é uma
// tradução fina de "o usuário clicou em X" para o módulo que faz o trabalho.
use crate::auth;
use crate::config;
use crate::state::{emit_status, AppState, StatusPayload};
use crate::update;
use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt;
use tracing::info;

/// Pareia o client com a conta do site (login lá é via Steam): troca o código
/// de conexão gerado no perfil por access/refresh tokens.
#[tauri::command]
pub async fn login(app: AppHandle, code: String) -> Result<(), String> {
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
pub async fn logout(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    state.tokens.clear().await;
    *state.active_match.lock().unwrap() = None;
    *state.last_snapshot.lock().unwrap() = None;
    *state.last_sync.lock().unwrap() = None;
    let _ = state.session_tx.send(false);
    info!("logout realizado");
    emit_status(&app);
    Ok(())
}

#[tauri::command]
pub async fn get_status(app: AppHandle) -> StatusPayload {
    // Garante que o cache de tokens foi consultado (primeira chamada da UI).
    let _ = app.state::<AppState>().tokens.load().await;
    crate::state::status_payload(&app)
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let manager = app.autolaunch();
    let resultado = if enabled { manager.enable() } else { manager.disable() };
    resultado.map_err(|e| e.to_string())?;
    emit_status(&app);
    Ok(())
}

#[tauri::command]
pub fn open_logs() {
    let dir = config::log_dir();
    let _ = std::fs::create_dir_all(&dir);
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg(&dir).spawn();
    }
}

/// Abre o instalador da versão nova no navegador padrão.
#[tauri::command]
pub fn baixar_atualizacao(app: AppHandle) -> Result<(), String> {
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
pub async fn verificar_atualizacao(app: AppHandle) {
    update::verificar(&app).await;
}
