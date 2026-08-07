mod analytics;
mod auth;
mod cli;
mod config;
mod content;
mod db;
mod error;
mod media_store;
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
    let config_path = cli.config_path();

    match cli.command {
        Command::Serve => {
            let config = Config::load(config_path.as_ref())?;
            config.warn_if_loopback_in_container();
            serve(config).await?;
        }
        Command::Init { dir, force, reset_password } => {
            cli::init::run(&dir, force, reset_password)?;
        }
        Command::Theme { dir, force } => {
            cli::init::theme(&dir, force)?;
        }
        Command::Migrate => {
            let config = Config::load(config_path.as_ref())?;
            let pool = db::open_pool(&config.server.database)?;
            let mut conn = pool.get()?;
            db::run_migrations(&mut conn)?;
            println!("migrations applied");
        }
        Command::Healthcheck => {
            let config = Config::load(config_path.as_ref())?;
            cli::healthcheck::run(&config)?;
        }
        Command::Import { dir, published, force, dry_run } => {
            let config = Config::load(config_path.as_ref())?;
            let pool = db::open_pool(&config.server.database)?;
            let mut conn = pool.get()?;
            db::run_migrations(&mut conn)?;
            let opts = cli::import::ImportOptions { published, force, dry_run };
            cli::import::run(&mut conn, &dir, &opts, &config.markdown)?;
        }
        Command::Export { dir } => {
            let config = Config::load(config_path.as_ref())?;
            let pool = db::open_pool(&config.server.database)?;
            let mut conn = pool.get()?;
            db::run_migrations(&mut conn)?;
            cli::export::run(&conn, &dir)?;
        }
        Command::Rerender => {
            let config = Config::load(config_path.as_ref())?;
            let pool = db::open_pool(&config.server.database)?;
            let mut conn = pool.get()?;
            db::run_migrations(&mut conn)?;
            cli::rerender::run(&conn, &config.markdown)?;
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
    let (analytics_handle, analytics_writer) =
        analytics::AnalyticsHandle::spawn(pool.clone(), &config.analytics).await?;
    let templates = std::sync::Arc::new(render::build_env()?);
    let state = web::AppState {
        config: std::sync::Arc::new(config),
        db: pool,
        cookie_key,
        login_ratelimit: std::sync::Arc::new(auth::ratelimit::RateLimiter::new()),
        analytics: std::sync::Arc::new(analytics_handle),
        page_cache: std::sync::Arc::new(render::cache::PageCache::new()),
        templates,
    };
    web::public::warm_cache(&state).await;
    let app = web::build_router(state);

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    tracing::info!("listening on {bind_addr}");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    // `app` (and every clone of AppState handed to in-flight requests) is
    // dropped by this point, so the analytics channel's sender side is gone
    // too — awaiting the writer here is what lets it flush its last batch
    // before the process exits.
    analytics_writer.await.ok();

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
