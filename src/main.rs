mod auth;
mod cli;
mod config;
mod content;
mod db;
mod error;
mod render;
mod web;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use cli::{Cli, Command};
use config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::HashPassword => {
            cli::hash_password::run()?;
        }
        Command::Serve => {
            let config = Config::load(cli.config.as_ref())?;
            config.warn_if_loopback_in_container();
            serve(config).await?;
        }
        Command::Migrate => {
            let config = Config::load(cli.config.as_ref())?;
            let pool = db::open_pool(&config.server.database)?;
            let mut conn = pool.get()?;
            db::run_migrations(&mut conn)?;
            println!("migrations applied");
        }
        Command::Healthcheck => {
            let config = Config::load(cli.config.as_ref())?;
            cli::healthcheck::run(&config)?;
        }
        Command::Import { dir, .. } => {
            eprintln!("import not yet implemented (dir: {})", dir.display());
            std::process::exit(1);
        }
        Command::Export { dir } => {
            eprintln!("export not yet implemented (dir: {})", dir.display());
            std::process::exit(1);
        }
        Command::Rerender => {
            eprintln!("rerender not yet implemented");
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn serve(config: Config) -> anyhow::Result<()> {
    let pool = db::open_pool(&config.server.database)?;
    {
        let mut conn = pool.get()?;
        db::run_migrations(&mut conn)?;
    }

    let bind_addr = config.bind_addr()?;
    let cookie_key = cookie::Key::derive_from(config.auth.session_secret.as_bytes());
    let state = web::AppState {
        config: std::sync::Arc::new(config),
        db: pool,
        cookie_key,
        login_ratelimit: std::sync::Arc::new(auth::ratelimit::RateLimiter::new()),
    };
    let app = web::build_router(state);

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    tracing::info!("listening on {bind_addr}");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install ctrl-c handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received, draining");
}
