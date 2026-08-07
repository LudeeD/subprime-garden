pub mod export;
pub mod hash_password;
pub mod healthcheck;
pub mod import;
pub mod init;
pub mod rerender;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "subprime-garden", version, about = "A single-user blogging engine")]
pub struct Cli {
    /// Path to the TOML config file. Falls back to $SUBPRIME_CONFIG, then
    /// ./garden.toml if that exists, then env vars alone.
    #[arg(long, global = true, env = "SUBPRIME_CONFIG")]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    /// --config / $SUBPRIME_CONFIG, else ./garden.toml if present.
    pub fn config_path(&self) -> Option<PathBuf> {
        self.config.clone().or_else(|| {
            let default = PathBuf::from("garden.toml");
            default.is_file().then_some(default)
        })
    }
}

#[derive(Subcommand)]
pub enum Command {
    /// Run the HTTP server.
    Serve,
    /// Scaffold templates/static and walk garden.toml to a complete config.
    Init {
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Overwrite existing files and re-run the full prompt sequence.
        #[arg(long)]
        force: bool,
        /// Prompt for a new admin password even if one is already set.
        #[arg(long)]
        reset_password: bool,
    },
    /// Scaffold or refresh templates/static only — never touches garden.toml.
    Theme {
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Overwrite existing template/static files with the stock theme.
        #[arg(long)]
        force: bool,
    },
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
