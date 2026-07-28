// config.rs — Config Manager: config.json em %APPDATA%\ResenhaClient.
// Os tokens de sessão NÃO ficam aqui (vão pro Credential Manager, ver auth.rs).
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;

pub const DEFAULT_GSI_PORT: u16 = 3210;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// URL base do backend dedicado (REST + WebSocket).
    pub backend_url: String,
    /// Porta do servidor HTTP local que recebe o GSI.
    pub gsi_port: u16,
    /// Token secreto compartilhado com o CS2 via gamestate_integration_resenha.cfg.
    /// Gerado uma vez na primeira execução e mantido estável (o .cfg é fixo).
    pub gsi_token: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            backend_url: "http://localhost:4000".into(),
            gsi_port: DEFAULT_GSI_PORT,
            gsi_token: String::new(),
        }
    }
}

impl AppConfig {
    /// Deriva a URL do WebSocket a partir da URL base do backend.
    /// http://x → ws://x/ws/client | https://x → wss://x/ws/client
    pub fn ws_url(&self) -> String {
        let base = self.backend_url.trim_end_matches('/');
        let ws_base = if let Some(rest) = base.strip_prefix("https://") {
            format!("wss://{rest}")
        } else if let Some(rest) = base.strip_prefix("http://") {
            format!("ws://{rest}")
        } else {
            format!("ws://{base}")
        };
        format!("{ws_base}/ws/client")
    }
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .expect("diretório de configuração do usuário não encontrado")
        .join("ResenhaClient")
}

pub fn log_dir() -> PathBuf {
    config_dir().join("logs")
}

fn config_file() -> PathBuf {
    config_dir().join("config.json")
}

pub fn load() -> AppConfig {
    let mut cfg: AppConfig = fs::read_to_string(config_file())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    if cfg.gsi_token.is_empty() {
        cfg.gsi_token = random_token();
        if let Err(e) = save(&cfg) {
            eprintln!("falha ao salvar config inicial: {e}");
        }
    }
    cfg
}

pub fn save(cfg: &AppConfig) -> io::Result<()> {
    fs::create_dir_all(config_dir())?;
    fs::write(config_file(), serde_json::to_string_pretty(cfg).unwrap())
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
