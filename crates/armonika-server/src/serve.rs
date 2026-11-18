//! The `serve` command: an HTTP server on `server.bind`.
//!
//! At startup it opens the database, creates the first admin if needed and creates the
//! libraries of the configuration file that do not exist yet. It then starts the scan
//! controller: a scan of every library, folder watching and periodic rescans.

use std::future::IntoFuture;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::Router;
use axum::routing::get;

use armonika_core::Error;
use armonika_core::db::Db;
use armonika_core::libraries;
use armonika_core::secret::SecretBox;
use armonika_core::users::{self, BootstrapOutcome};
use armonika_scanner::controller::ScanController;
use armonika_scanner::fingerprinter::{FingerprintOptions, ffmpeg_or_default};
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

/// How long shutdown waits for each of: the scans to end their batch, the scrobble
/// outbox pass in progress, the HTTP server to finish its requests and the search
/// index updater to apply its last changes.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);

/// How often shutdown looks whether the scans have ended.
const STOP_POLL: Duration = Duration::from_millis(50);

/// Startup order: the store (`data_dir`, database, secret key), the first admin, the
/// libraries of the configuration file, the search index (rebuilt in the background
/// when it is missing or outdated: the API answers at once and search is empty until
/// it is done), the scan controller with the content hasher and the folder watchers,
/// the scrobble outbox, then the HTTP server.
///
/// Shutdown order: watchers, scans (each ends the batch it is writing), the hasher and
/// the outbox, then the HTTP server (at most [`STOP_TIMEOUT`] for open requests), then
/// the search index updater. A failure after the background parts started (the
/// address is taken, the server breaks) stops them in the same order before the error
/// is returned.
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
    let mut background = Background {
        controller: controller.clone(),
        watcher: Some(watcher),
        search_updater,
        stop_outbox,
        outbox_task,
    };
    let state = app_state(&config, &db, secrets, search, controller, sink);
    let result = match Http::start(&config, state).await {
        Ok(mut http) => {
            let ended = tokio::select! {
                () = shutdown_signal() => None,
                ended = &mut http.server => Some(server_outcome(ended)),
            };
            background.halt().await;
            match ended {
                Some(result) => result,
                None => http.stop().await,
            }
        }
        Err(e) => {
            background.halt().await;
            Err(e)
        }
    };
    background.finish(&db).await;
    tracing::info!("shutdown complete");
    result
}

/// The HTTP server running on its own task.
struct Http {
    server: tokio::task::JoinHandle<std::io::Result<()>>,
    stop: tokio::sync::oneshot::Sender<()>,
}

impl Http {
    async fn start(config: &Config, state: AppState) -> anyhow::Result<Http> {
        let listener = tokio::net::TcpListener::bind(config.server.bind)
            .await
            .with_context(|| format!("cannot bind {}", config.server.bind))?;
        tracing::info!(bind = %config.server.bind, "armonika listening");
        let (stop, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(
            axum::serve(listener, router(config, state))
                .with_graceful_shutdown(async move {
                    let _ = stop_rx.await;
                })
                .into_future(),
        );
        Ok(Http { server, stop })
    }

    /// Stops accepting requests and gives the open ones [`STOP_TIMEOUT`].
    async fn stop(mut self) -> anyhow::Result<()> {
        let _ = self.stop.send(());
        match tokio::time::timeout(STOP_TIMEOUT, &mut self.server).await {
            Ok(ended) => server_outcome(ended),
            Err(_) => {
                tracing::warn!("open requests did not finish in time, closing them");
                self.server.abort();
                Ok(())
            }
        }
    }
}

fn server_outcome(
    ended: Result<std::io::Result<()>, tokio::task::JoinError>,
) -> anyhow::Result<()> {
    ended
        .context("the server task failed")?
        .context("server error")
}

/// Everything `run` starts besides the HTTP server.
struct Background {
    controller: ScanController,
    watcher: Option<watch::WatchHandle>,
    search_updater: tokio::task::JoinHandle<()>,
    stop_outbox: tokio::sync::watch::Sender<bool>,
    outbox_task: tokio::task::JoinHandle<()>,
}

impl Background {
    /// Stops the folder watchers, the scans and the hasher, and the scrobble outbox.
    async fn halt(&mut self) {
        drop(self.watcher.take());
        self.controller.shutdown();
        let deadline = tokio::time::Instant::now() + STOP_TIMEOUT;
        while self.controller.status().scanning {
            if tokio::time::Instant::now() >= deadline {
                tracing::warn!("a scan did not stop in time");
                break;
            }
            tokio::time::sleep(STOP_POLL).await;
        }
        let _ = self.stop_outbox.send(true);
        if tokio::time::timeout(STOP_TIMEOUT, &mut self.outbox_task)
            .await
            .is_err()
        {
            // Rows are only deleted after the service took them: what is cut short is
            // sent again at the next start.
            tracing::warn!("the scrobble outbox did not stop in time");
        }
    }

    /// Waits for the search index updater. Call it once the HTTP server is gone: the
    /// scan controller holds the senders of the change channel, and when the last
    /// handle is dropped the updater applies what is pending and ends. Changes it could
    /// not apply in time are lost for good (the scanner reports each only once), so the
    /// index is marked as outdated and the next start rebuilds it.
    async fn finish(self, db: &Db) {
        drop(self.controller);
        match tokio::time::timeout(STOP_TIMEOUT, self.search_updater).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::error!(error = %e, "the search index updater failed"),
            Err(_) => {
                tracing::warn!("the search index updater did not finish in time");
                updater::invalidate(db).await;
            }
        }
    }
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
    let fingerprint = config
        .recognition
        .background_fingerprint
        .then(|| FingerprintOptions {
            // Like the transcoder: without a configured path, ffmpeg is looked up in PATH.
            ffmpeg: Some(ffmpeg_or_default(config.ffmpeg_path.clone())),
        });
    let controller = ScanController::new(db.clone(), opts, changes, fingerprint);
    let libraries = db
        .read(libraries::list)
        .await
        .context("cannot list the libraries")?;
    let watched = if config.scan.watch {
        libraries.into_iter().map(|l| (l.id, l.root)).collect()
    } else {
        Vec::new()
    };
    // Setting up the watchers walks the folders on some platforms: not on a worker.
    let (watch_controller, interval) = (controller.clone(), config.scan.interval);
    let watcher =
        tokio::task::spawn_blocking(move || watch::spawn(watch_controller, watched, interval))
            .await
            .context("watcher task failed")?;
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
