pub mod hash_password;
pub mod healthcheck;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "subprime-garden", version, about = "A single-user blogging engine")]
pub struct Cli {
    /// Path to the TOML config file. Falls back to $SUBPRIME_CONFIG, then env vars alone.
    #[arg(long, global = true, env = "SUBPRIME_CONFIG")]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run the HTTP server.
    Serve,
    /// Prompt for a password and print an argon2 hash to paste into config.
    HashPassword,
    /// Run pending database migrations and exit.
    Migrate,
    /// Import a directory of markdown files (Zola/Hugo/Jekyll frontmatter accepted).
    Import {
        dir: PathBuf,
        #[arg(long)]
        published: bool,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Export the database back out as markdown files with YAML frontmatter.
    Export { dir: PathBuf },
    /// Rebuild stored HTML for every post (after markdown option changes).
    Rerender,
    /// Internal: check /healthz over a raw TCP connection. Used by the container healthcheck.
    Healthcheck,
}
