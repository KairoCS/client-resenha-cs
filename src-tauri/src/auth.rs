// auth.rs — Authentication + Token Manager.
// Tokens ficam no Windows Credential Manager (keyring), nunca em arquivo.
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::warn;

const SERVICE: &str = "ResenhaClient";
const ACCESS_KEY: &str = "access_token";
const REFRESH_KEY: &str = "refresh_token";

#[derive(Clone)]
pub struct Tokens {
    pub access: String,
    pub refresh: String,
}

pub struct TokenManager {
    cache: tokio::sync::Mutex<Option<Tokens>>,
    logged_in: AtomicBool,
}

impl TokenManager {
    pub fn new() -> Self {
        Self {
            cache: tokio::sync::Mutex::new(None),
            logged_in: AtomicBool::new(false),
        }
    }

    fn entry(key: &str) -> Option<keyring::Entry> {
        keyring::Entry::new(SERVICE, key)
            .map_err(|e| warn!("keyring indisponível: {e}"))
            .ok()
    }

    /// Leitura síncrona do estado (pra tray/status sem async).
    pub fn is_logged_in(&self) -> bool {
        self.logged_in.load(Ordering::Relaxed)
    }

    /// Carrega os tokens (Credential Manager na primeira vez, depois cache).
    pub async fn load(&self) -> Option<Tokens> {
        let mut cache = self.cache.lock().await;
        if cache.is_none() {
            let access = Self::entry(ACCESS_KEY)?.get_password().ok()?;
            let refresh = Self::entry(REFRESH_KEY)?.get_password().ok()?;
            *cache = Some(Tokens { access, refresh });
        }
        self.logged_in.store(cache.is_some(), Ordering::Relaxed);
        cache.clone()
    }

    pub async fn store(&self, tokens: Tokens) {
        if let Some(e) = Self::entry(ACCESS_KEY) {
            if let Err(err) = e.set_password(&tokens.access) {
                warn!("falha ao salvar access token: {err}");
            }
        }
        if let Some(e) = Self::entry(REFRESH_KEY) {
            if let Err(err) = e.set_password(&tokens.refresh) {
                warn!("falha ao salvar refresh token: {err}");
            }
        }
        *self.cache.lock().await = Some(tokens);
        self.logged_in.store(true, Ordering::Relaxed);
    }

    pub async fn clear(&self) {
        for key in [ACCESS_KEY, REFRESH_KEY] {
            if let Some(e) = Self::entry(key) {
                let _ = e.delete_credential();
            }
        }
        *self.cache.lock().await = None;
        self.logged_in.store(false, Ordering::Relaxed);
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
}

pub enum AuthError {
    /// Credenciais/refresh token rejeitados pelo backend — sessão inválida.
    Unauthorized,
    /// Rede fora, servidor caído etc. — vale tentar de novo depois.
    Other(String),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::Unauthorized => write!(f, "código inválido ou expirado"),
            AuthError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("reqwest client")
}

async fn token_request(url: String, body: serde_json::Value) -> Result<Tokens, AuthError> {
    let resp = http()
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| AuthError::Other(format!("falha de conexão com o servidor: {e}")))?;

    match resp.status().as_u16() {
        200 | 201 => {
            let t: TokenResponse = resp
                .json()
                .await
                .map_err(|e| AuthError::Other(format!("resposta inválida do servidor: {e}")))?;
            Ok(Tokens {
                access: t.access_token,
                refresh: t.refresh_token,
            })
        }
        401 | 403 => Err(AuthError::Unauthorized),
        s => Err(AuthError::Other(format!("erro do servidor (HTTP {s})"))),
    }
}

/// Pareamento: o usuário loga no site com a Steam, gera um código curto no
/// perfil e digita no client. O backend troca o código por tokens.
///
/// POST {backend}/api/client/auth/pair { code } → { access_token, refresh_token }
pub async fn pair(backend_url: &str, code: &str) -> Result<Tokens, AuthError> {
    let base = backend_url.trim_end_matches('/');
    token_request(
        format!("{base}/api/client/auth/pair"),
        serde_json::json!({ "code": code }),
    )
    .await
}

/// POST {backend}/api/client/auth/refresh { refresh_token } → novo par de tokens
pub async fn refresh(backend_url: &str, refresh_token: &str) -> Result<Tokens, AuthError> {
    let base = backend_url.trim_end_matches('/');
    token_request(
        format!("{base}/api/client/auth/refresh"),
        serde_json::json!({ "refresh_token": refresh_token }),
    )
    .await
}
