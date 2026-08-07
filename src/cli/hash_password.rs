use anyhow::Result;
use argon2::password_hash::{PasswordHasher, SaltString};
use argon2::Argon2;
use rand_core::OsRng;

/// Used by `init` to turn a prompted plaintext password into what
/// `[auth] password_hash` actually stores.
pub fn hash(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("failed to hash password: {e}"))?;
    Ok(hash.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use argon2::password_hash::{PasswordHash, PasswordVerifier};

    #[test]
    fn hash_roundtrips_and_looks_like_argon2() {
        let hashed = hash("testpass123").unwrap();
        assert!(hashed.starts_with("$argon2"));

        let parsed = PasswordHash::new(&hashed).unwrap();
        assert!(Argon2::default()
            .verify_password(b"testpass123", &parsed)
            .is_ok());
        assert!(Argon2::default()
            .verify_password(b"wrong", &parsed)
            .is_err());
    }
}
