use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use rust_embed::Embed;

use crate::cli::hash_password;
use crate::config::{PLACEHOLDER_PASSWORD_HASH, PLACEHOLDER_SESSION_SECRET};

#[derive(Embed)]
#[folder = "src/render/default_templates/"]
struct StockTemplates;

#[derive(Embed)]
#[folder = "src/render/default_static/"]
struct StockStatic;

const GARDEN_TOML_EXAMPLE: &str = include_str!("../../garden.toml.example");

pub fn run(dir: &Path, force: bool, reset_password: bool) -> Result<()> {
    theme(dir, force)?;

    let config_path = dir.join("garden.toml");
    configure(&config_path, force, reset_password)?;

    println!(
        "\nYour site is configured at {}. Run `subprime-garden serve` (or `docker compose up -d`) to start.",
        config_path.display()
    );
    Ok(())
}

/// Scaffolds templates/static only — used standalone by `theme` and as the
/// first half of `init`.
pub fn theme(dir: &Path, force: bool) -> Result<()> {
    let mut written = 0;
    let mut skipped = 0;

    for name in StockTemplates::iter() {
        let file = StockTemplates::get(&name).expect("just listed by iter()");
        write_file(&dir.join("templates").join(name.as_ref()), &file.data, force, &mut written, &mut skipped)?;
    }
    for name in StockStatic::iter() {
        let file = StockStatic::get(&name).expect("just listed by iter()");
        write_file(&dir.join("static").join(name.as_ref()), &file.data, force, &mut written, &mut skipped)?;
    }
    println!("{written} written, {skipped} skipped");
    Ok(())
}

/// Returns whether the file was actually written (vs. skipped).
fn write_file(path: &Path, data: &[u8], force: bool, written: &mut u32, skipped: &mut u32) -> Result<bool> {
    if path.exists() && !force {
        println!("skipped:   {} (already exists, use --force to overwrite)", path.display());
        *skipped += 1;
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, data)?;
    println!("wrote:     {}", path.display());
    *written += 1;
    Ok(true)
}

fn configure(config_path: &Path, force: bool, reset_password: bool) -> Result<()> {
    let existing = std::fs::read_to_string(config_path).unwrap_or_default();
    let fresh = force || existing.trim().is_empty();

    let base = if fresh { GARDEN_TOML_EXAMPLE } else { existing.as_str() };
    let mut doc: toml_edit::DocumentMut = base.parse().context("failed to parse config as TOML")?;

    if fresh {
        for (env_var, table, key, label, default) in [
            ("SUBPRIME_SITE__TITLE", "site", "title", "Site title", "subprime garden"),
            ("SUBPRIME_SITE__DESCRIPTION", "site", "description", "Site description (optional)", ""),
            ("SUBPRIME_SITE__AUTHOR", "site", "author", "Author name", ""),
            ("SUBPRIME_SITE__BASE_URL", "site", "base_url", "Base URL (e.g. https://blog.example.com)", "http://localhost:8080"),
            ("SUBPRIME_AUTH__USERNAME", "auth", "username", "Admin username", "owner"),
        ] {
            let value = resolve(env_var, label, default)?;
            doc[table][key] = toml_edit::value(value);
        }
    }

    resolve_password(&mut doc, reset_password)?;
    resolve_session_secret(&mut doc)?;

    std::fs::write(config_path, doc.to_string())?;
    Ok(())
}

fn resolve(env_var: &str, label: &str, default: &str) -> Result<String> {
    match std::env::var(env_var) {
        Ok(v) if !v.trim().is_empty() => Ok(v),
        _ => prompt(label, default),
    }
}

fn prompt(label: &str, default: &str) -> Result<String> {
    if default.is_empty() {
        print!("{label}: ");
    } else {
        print!("{label} [{default}]: ");
    }
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let trimmed = line.trim();
    Ok(if trimmed.is_empty() { default.to_string() } else { trimmed.to_string() })
}

fn needs_secret(current: &str, placeholder: &str) -> bool {
    let current = current.trim();
    current.is_empty() || current == placeholder
}

fn resolve_password(doc: &mut toml_edit::DocumentMut, force: bool) -> Result<()> {
    let current = doc["auth"]["password_hash"].as_str().unwrap_or("").to_string();
    if !force && !needs_secret(&current, PLACEHOLDER_PASSWORD_HASH) {
        return Ok(());
    }
    if !force {
        if let Ok(v) = std::env::var("SUBPRIME_AUTH__PASSWORD_HASH") {
            if v.starts_with("$argon2") {
                doc["auth"]["password_hash"] = toml_edit::value(v);
                return Ok(());
            }
        }
    }

    let password = rpassword::prompt_password("Admin password: ").context("failed to read password")?;
    let confirm = rpassword::prompt_password("Confirm password: ").context("failed to read password")?;
    if password != confirm {
        anyhow::bail!("passwords did not match");
    }
    if password.is_empty() {
        anyhow::bail!("password must not be empty");
    }
    doc["auth"]["password_hash"] = toml_edit::value(hash_password::hash(&password)?);
    Ok(())
}

fn resolve_session_secret(doc: &mut toml_edit::DocumentMut) -> Result<()> {
    let current = doc["auth"]["session_secret"].as_str().unwrap_or("").to_string();
    if !needs_secret(&current, PLACEHOLDER_SESSION_SECRET) {
        return Ok(());
    }
    let value = std::env::var("SUBPRIME_AUTH__SESSION_SECRET")
        .ok()
        .filter(|v| v.len() >= 32)
        .unwrap_or_else(crate::auth::random_token);
    doc["auth"]["session_secret"] = toml_edit::value(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stock templates `theme`/`init` scaffold onto disk must parse as
    /// valid minijinja alongside the baked-in admin templates, since a fresh
    /// site combines both (site templates from disk, admin from the binary).
    #[test]
    fn stock_templates_parse() {
        let mut env = minijinja::Environment::new();
        for name in StockTemplates::iter() {
            let file = StockTemplates::get(&name).expect("just listed by iter()");
            let source = std::str::from_utf8(&file.data).expect("stock template is valid utf-8").to_string();
            env.add_template_owned(name.to_string(), source).expect("every stock template should parse");
        }
        crate::render::admin_assets::register(&mut env).expect("admin templates should parse");
    }
}
