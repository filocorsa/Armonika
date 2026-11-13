//! The `serve` command: an HTTP server on `server.bind`.
//!
//! At startup it opens the database, creates the first admin if needed and creates the
//! libraries of the configuration file that do not exist yet.

use anyhow::Context;
use axum::Router;
use axum::routing::get;

use armonika_core::Error;
use armonika_core::db::Db;
use armonika_core::libraries;
use armonika_core::secret::SecretBox;
use armonika_core::users::{self, BootstrapOutcome};

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
    let (db, secrets) = crate::cli::open_store(&config)?;
    bootstrap_admin(&db, &secrets).await?;
    sync_libraries(&db, &config).await?;
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

/// Creates the first admin from `ARMONIKA_ADMIN_USER` and `ARMONIKA_ADMIN_PASSWORD`
/// when no admin exists, and logs what happened (never the values). A refused name or
/// password is logged as an error and the server keeps running: an admin can still be
/// created with `armonika user create`.
async fn bootstrap_admin(db: &Db, secrets: &SecretBox) -> anyhow::Result<()> {
    let name = env_value("ARMONIKA_ADMIN_USER");
    let password = env_value("ARMONIKA_ADMIN_PASSWORD");
    let secrets = secrets.clone();
    let requested = name.clone();
    let outcome = db
        .write(move |c| users::bootstrap_admin(c, &secrets, name, password))
        .await;
    match outcome {
        Ok(BootstrapOutcome::Created(_)) => {
            tracing::info!(
                user = requested.as_deref().unwrap_or_default(),
                "created the admin from ARMONIKA_ADMIN_USER and ARMONIKA_ADMIN_PASSWORD"
            );
        }
        Ok(BootstrapOutcome::AdminExists) => {
            tracing::debug!("an admin exists; ARMONIKA_ADMIN_USER is not used");
        }
        Ok(BootstrapOutcome::MissingCredentials) => {
            tracing::warn!(
                "no admin exists: set ARMONIKA_ADMIN_USER and ARMONIKA_ADMIN_PASSWORD and                  restart, or run `armonika user create <name> --admin`"
            );
        }
        Err(Error::UserExists(user)) => {
            tracing::error!(
                user,
                "cannot create the admin from ARMONIKA_ADMIN_USER: a user with this name                  exists and is not an admin. Choose another name, or make that user an                  admin with `armonika user set <name> --admin true`"
            );
        }
        Err(e @ (Error::PasswordTooShort { .. } | Error::InvalidUserName(_))) => {
            tracing::error!(
                "cannot create the admin from ARMONIKA_ADMIN_USER and                  ARMONIKA_ADMIN_PASSWORD: {e}"
            );
        }
        Err(e) => return Err(e).context("cannot create the admin"),
    }
    Ok(())
}

/// Creates the libraries of the configuration file that are not in the database yet,
/// and logs as warnings what it did not apply.
async fn sync_libraries(db: &Db, config: &Config) -> anyhow::Result<()> {
    let configured = config.libraries.clone();
    let data_dir = config.data_dir.clone();
    let notes = db
        .write(move |c| libraries::sync_from_config(c, &configured, &data_dir))
        .await
        .context("cannot create the configured libraries")?;
    for note in notes {
        tracing::warn!("{note}");
    }
    Ok(())
}

/// The value of the variable `name`, if set and not empty.
fn env_value(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Resolves on Ctrl+C or (unix) SIGTERM. A handler that cannot be installed
/// is logged and never resolves, so the server keeps running instead of
/// shutting down at once.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::error!(%err, "cannot listen for Ctrl+C; stop the process another way");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(err) => {
                tracing::warn!(%err, "cannot listen for SIGTERM");
                std::future::pending::<()>().await
            }
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
    tracing::info!("shutdown signal received");
}
