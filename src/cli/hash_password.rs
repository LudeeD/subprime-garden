use anyhow::{Context, Result};
use argon2::password_hash::{PasswordHasher, SaltString};
use argon2::Argon2;
use rand_core::OsRng;

pub fn run() -> Result<()> {
    let password = rpassword::prompt_password("Password: ").context("failed to read password")?;
    let confirm = rpassword::prompt_password("Confirm: ").context("failed to read password")?;
    if password != confirm {
        anyhow::bail!("passwords did not match");
    }
    if password.is_empty() {
        anyhow::bail!("password must not be empty");
    }

    println!("{}", hash(&password)?);
    Ok(())
}

fn hash(password: &str) -> Result<String> {
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
