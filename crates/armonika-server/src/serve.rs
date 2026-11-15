//! The `serve` command: an HTTP server on `server.bind`.
//!
//! At startup it opens the database, creates the first admin if needed and creates the
//! libraries of the configuration file that do not exist yet. It then starts the scan
//! controller: a scan of every library, folder watching and periodic rescans.

use std::sync::Arc;

use anyhow::Context;
use axum::Router;
use axum::routing::get;

use armonika_core::Error;
use armonika_core::db::Db;
use armonika_core::libraries;
use armonika_core::secret::SecretBox;
use armonika_core::users::{self, BootstrapOutcome};
use armonika_scanner::controller::ScanController;
use armonika_scanner::{ScanOptions, watch};
use armonika_scrobble::listenbrainz::ListenBrainz;
use armonika_scrobble::{ScrobbleService, ScrobbleSink, outbox};
use armonika_search::{SearchIndex, TantivyIndex, updater};
use armonika_subsonic::{AppState, MediaConfig};

use crate::config::Config;

fn router(config: &Config, state: AppState) -> Router {
    let api = armonika_subsonic::router(state);
    if config.web.enabled {
        api.route(
            "/",
            get(|| async { format!("Armonika {}", armonika_core::VERSION) }),
        )
    } else {
        api
    }
}

fn app_state(
    config: &Config,
    db: &Db,
    secrets: SecretBox,
    search: Arc<dyn SearchIndex>,
    scans: ScanController,
    scrobbles: ScrobbleSink,
) -> AppState {
    AppState {
        db: db.clone(),
        secrets: Arc::new(secrets),
        search,
        scans,
        media: MediaConfig {
            ffmpeg: config.ffmpeg_path.clone(),
            transcode_format: config.transcode.format.clone(),
            transcode_bitrate_kbps: config.transcode.bitrate_kbps,
            cover_cache_dir: config.data_dir.join("covers"),
        },
        scrobbles,
        ignored_articles: config
            .search
            .ignored_articles
            .split_whitespace()
            .map(str::to_owned)
            .collect(),
    }
}

/// How long shutdown waits for the scrobble outbox pass in progress.
const OUTBOX_STOP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// How long shutdown waits for the search index updater to apply its last changes.
const SEARCH_FLUSH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

pub async fn run(config: Config) -> anyhow::Result<()> {
    let (db, secrets) = crate::cli::open_store(&config)?;
    bootstrap_admin(&db, &secrets).await?;
    sync_libraries(&db, &config).await?;
    let (controller, watcher, search_updater, search) = start_scanning(&db, &config).await?;
    let services: Vec<Arc<dyn ScrobbleService>> =
        vec![Arc::new(ListenBrainz::new(&config.listenbrainz.url))];
    let sink = ScrobbleSink::new(db.clone(), secrets.clone(), services.clone());
    let (stop_outbox, stop_rx) = tokio::sync::watch::channel(false);
    let outbox_task = tokio::spawn(outbox::run(db.clone(), secrets.clone(), services, stop_rx));
    let state = app_state(&config, &db, secrets, search, controller.clone(), sink);
    let listener = tokio::net::TcpListener::bind(config.server.bind)
        .await
        .with_context(|| format!("cannot bind {}", config.server.bind))?;
    tracing::info!(bind = %config.server.bind, "armonika listening");
    axum::serve(listener, router(&config, state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;
    drop(watcher);
    let _ = stop_outbox.send(true);
    if tokio::time::timeout(OUTBOX_STOP_TIMEOUT, outbox_task)
        .await
        .is_err()
    {
        // Rows are only deleted after the service took them: what is cut short is
        // sent again at the next start.
        tracing::warn!("the scrobble outbox did not stop in time");
    }
    controller.shutdown();
    // The scan controller holds the senders of the change channel: once it is gone the
    // updater applies what is pending and ends. Changes it could not apply in time
    // are lost for good (the scanner reports each only once), so mark the index
    // as outdated and let the next start rebuild it.
    drop(controller);
    match tokio::time::timeout(SEARCH_FLUSH_TIMEOUT, search_updater).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::error!(error = %e, "the search index updater failed"),
        Err(_) => {
            tracing::warn!("the search index updater did not finish in time");
            updater::invalidate(&db).await;
        }
    }
    tracing::info!("shutdown complete");
    Ok(())
}

/// Starts the scan controller, asks for a scan of every library and starts the folder
/// watchers and the periodic rescan. The changes the scans report go to the search
/// index updater, which first rebuilds the index if it is missing or outdated.
async fn start_scanning(
    db: &Db,
    config: &Config,
) -> anyhow::Result<(
    ScanController,
    watch::WatchHandle,
    tokio::task::JoinHandle<()>,
    Arc<dyn SearchIndex>,
)> {
    let (changes, rx) = tokio::sync::mpsc::unbounded_channel();
    let search_dir = config.data_dir.join("search");
    let index = tokio::task::spawn_blocking(move || TantivyIndex::open(&search_dir))
        .await
        .context("search index task failed")?
        .context("cannot open the search index")?;
    let created = index.was_created();
    let index: Arc<dyn SearchIndex> = Arc::new(index);
    let search_db = db.clone();
    let updater_index = index.clone();
    let search_updater = tokio::spawn(async move {
        let index = updater_index;
        if let Err(e) = updater::rebuild_if_needed(search_db.clone(), index.clone(), created).await
        {
            tracing::error!(error = %e, "cannot rebuild the search index");
        }
        updater::run(search_db, index, rx).await;
    });
    let defaults = ScanOptions::default();
    let opts = ScanOptions {
        threads: config.scan.threads.max(1),
        ..defaults
    };
    let controller = ScanController::new(db.clone(), opts, changes);
    let libraries = db
        .read(libraries::list)
        .await
        .context("cannot list the libraries")?;
    let watched = if config.scan.watch {
        libraries.into_iter().map(|l| (l.id, l.root)).collect()
    } else {
        Vec::new()
    };
    let watcher = watch::spawn(controller.clone(), watched, config.scan.interval);
    let first = controller.clone();
    tokio::task::spawn_blocking(move || first.request_all());
    Ok((controller, watcher, search_updater, index))
}

/// Creates the first admin from `ARMONIKA_ADMIN_USER` and `ARMONIKA_ADMIN_PASSWORD`
/// when no admin exists, and logs what happened (never the values). A refused name or
/// password is logged as an error and the server keeps running: an admin can still be
/// created with `armonika user create`.
async fn bootstrap_admin(db: &Db, secrets: &SecretBox) -> anyhow::Result<()> {
    let name = env_value("ARMONIKA_ADMIN_USER");
    let password = env_value("ARMONIKA_ADMIN_PASSWORD");
    let env_set = name.is_some() || password.is_some();
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
        Ok(BootstrapOutcome::AdminExists) if env_set => {
            tracing::info!(
                "ARMONIKA_ADMIN_USER or ARMONIKA_ADMIN_PASSWORD is set but ignored because an \
                 admin already exists; to change a password run \
                 `armonika user set-password <name>`"
            );
        }
        Ok(BootstrapOutcome::AdminExists) => {
            tracing::debug!("an admin exists");
        }
        Ok(BootstrapOutcome::MissingCredentials) => {
            tracing::warn!(
                "no admin exists: set ARMONIKA_ADMIN_USER and ARMONIKA_ADMIN_PASSWORD and \
                 restart, or run `armonika user create <name> --admin`"
            );
        }
        Err(Error::UserExists(user)) => {
            tracing::error!(
                user,
                "cannot create the admin from ARMONIKA_ADMIN_USER: a user with this name \
                 exists and is not an admin. Choose another name, or make that user an \
                 admin with `armonika user set <name> --admin true`"
            );
        }
        Err(e @ (Error::PasswordTooShort { .. } | Error::InvalidUserName(_))) => {
            tracing::error!(
                "cannot create the admin from ARMONIKA_ADMIN_USER and \
                 ARMONIKA_ADMIN_PASSWORD: {e}"
            );
        }
        Err(e) => return Err(e).context("cannot create the admin"),
    }
    Ok(())
}

/// Creates the libraries of the configuration file that are not in the database yet,
/// logs each one created at info level and, as warnings, what it did not apply.
async fn sync_libraries(db: &Db, config: &Config) -> anyhow::Result<()> {
    let configured = config.libraries.clone();
    let data_dir = config.data_dir.clone();
    let notes = db
        .write(move |c| libraries::sync_from_config(c, &configured, &data_dir))
        .await
        .context("cannot create the configured libraries")?;
    for note in notes {
        if note.is_warning() {
            tracing::warn!("{note}");
        } else {
            tracing::info!("{note}");
        }
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
