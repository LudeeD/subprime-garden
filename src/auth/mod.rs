pub mod cookie;
pub mod ratelimit;

use argon2::password_hash::PasswordHash;
use argon2::{Argon2, PasswordVerifier};
use serde::{Deserialize, Serialize};

use crate::config::AuthConfig;

pub const SESSION_COOKIE: &str = "session";
pub const LOGIN_CSRF_COOKIE: &str = "login_csrf";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionData {
    pub issued_at: i64,
    pub csrf: String,
}

impl SessionData {
    pub fn new() -> Self {
        SessionData {
            issued_at: chrono::Utc::now().timestamp(),
            csrf: random_token(),
        }
    }

    pub fn is_expired(&self, ttl_days: u32) -> bool {
        let ttl_seconds = i64::from(ttl_days) * 86_400;
        let age = chrono::Utc::now().timestamp() - self.issued_at;
        age > ttl_seconds || age < 0
    }
}

/// 32 random bytes, hex-encoded — used for both session and login CSRF tokens.
pub fn random_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Verifies `username`/`password` against the single configured identity.
/// Argon2's own comparison is constant-time; the username check does not
/// need to be, since it isn't a secret.
pub fn verify_credentials(auth: &AuthConfig, username: &str, password: &str) -> bool {
    if username != auth.username {
        return false;
    }
    let Ok(hash) = PasswordHash::new(&auth.password_hash) else {
        tracing::error!("auth.password_hash is not a valid argon2 hash");
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &hash)
        .is_ok()
}
