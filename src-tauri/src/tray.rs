// tray.rs — Tray Manager: ícone na bandeja, menu de contexto e "fechar = esconder".
// Clique esquerdo abre o painel; clique direito abre o menu (padrão Discord/Steam).
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt;
use tracing::warn;

pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Abrir painel", true, None::<&str>)?;
    let autostart_checked = app.autolaunch().is_enabled().unwrap_or(false);
    let autostart = CheckMenuItem::with_id(
        app,
        "autostart",
        "Executar ao iniciar o Windows",
        true,
        autostart_checked,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &autostart, &separator, &quit])?;

    let autostart_item = autostart.clone();
    TrayIconBuilder::with_id("resenha-tray")
        .icon(app.default_window_icon().expect("ícone do app").clone())
        .tooltip("Resenha Client")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "open" => show_window(app),
            "autostart" => {
                // O CheckMenuItem já alternou visualmente; sincroniza o registro.
                let enable = autostart_item.is_checked().unwrap_or(false);
                let manager = app.autolaunch();
                let result = if enable { manager.enable() } else { manager.disable() };
                if let Err(e) = result {
                    warn!("falha ao configurar autostart: {e}");
                }
                crate::state::emit_status(app);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}
