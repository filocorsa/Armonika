//! The `serve` command: an HTTP server on `server.bind`.

use anyhow::Context;
use axum::Router;
use axum::routing::get;

use crate::config::Config;

fn router(config: &Config) -> Router {
    if config.web.enabled {
        Router::new().route(
            "/",
            get(|| async { format!("Armonika {}", armonika_core::VERSION) }),
        )
    } else {
        Router::new()
    }
}

pub async fn run(config: Config) -> anyhow::Result<()> {
    std::fs::create_dir_all(&config.data_dir)
        .with_context(|| format!("cannot create data dir {}", config.data_dir.display()))?;
    let listener = tokio::net::TcpListener::bind(config.server.bind)
        .await
        .with_context(|| format!("cannot bind {}", config.server.bind))?;
    tracing::info!(bind = %config.server.bind, "armonika listening");
    axum::serve(listener, router(&config))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;
    tracing::info!("shutdown complete");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
}
