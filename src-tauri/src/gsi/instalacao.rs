// gsi/instalacao.rs — coloca o gamestate_integration_resenha.cfg na pasta do
// CS2. É o que faz o jogo começar a postar o estado no nosso servidor local.
//
// O arquivo é FIXO: fica lá pra sempre depois de instalado (o CS2 só lê os
// gamestate_integration_*.cfg ao iniciar). Só reescrevemos se o conteúdo
// divergir — trocar o token à toa obrigaria a reiniciar o jogo.
use std::fs;
use std::path::PathBuf;
use tracing::info;

const GSI_FILE: &str = "gamestate_integration_resenha.cfg";

/// Formato KeyValues da Valve.
fn render_cfg(token: &str, port: u16) -> String {
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

    let atualizado = fs::read_to_string(&path)
        .map(|atual| atual == content)
        .unwrap_or(false);
    if !atualizado {
        fs::write(&path, &content).map_err(|e| format!("sem permissão pra escrever o .cfg: {e}"))?;
        info!("GSI instalado em {}", path.display());
    }
    Ok(path)
}

/// Steam → libraryfolders.vdf → biblioteca que contém o app 730 (CS2).
fn find_cs2_cfg_dir() -> Option<PathBuf> {
    let steam = steam_path()?;
    let mut bibliotecas = vec![steam.clone()];

    if let Ok(vdf) = fs::read_to_string(steam.join("steamapps").join("libraryfolders.vdf")) {
        for linha in vdf.lines() {
            let linha = linha.trim();
            if !linha.starts_with("\"path\"") {
                continue;
            }
            // formato:  "path"		"D:\\SteamLibrary"
            let partes: Vec<&str> = linha.split('"').collect();
            if let Some(raw) = partes.get(3) {
                bibliotecas.push(PathBuf::from(raw.replace("\\\\", "\\")));
            }
        }
    }

    bibliotecas.into_iter().find_map(|lib| {
        let steamapps = lib.join("steamapps");
        if !steamapps.join("appmanifest_730.acf").exists() {
            return None;
        }
        let cfg = steamapps
            .join("common")
            .join("Counter-Strike Global Offensive")
            .join("game")
            .join("csgo")
            .join("cfg");
        cfg.exists().then_some(cfg)
    })
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
