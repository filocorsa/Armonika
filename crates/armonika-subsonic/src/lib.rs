//! The OpenSubsonic API of the Armonika music server, served under `/rest`.

mod auth;
mod error;
mod response;
mod system;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use armonika_core::db::Db;
use armonika_core::secret::SecretBox;
use armonika_scanner::controller::ScanController;
use armonika_search::SearchIndex;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Path, State};
use axum::http::{Method, Request, header};
use axum::response::Response;
use axum::routing::get;

pub use auth::{Caller, authenticate};
pub use error::ApiError;
pub use response::{Format, Payload};

/// How long the answer to a wrong login is held back, to slow down guessing.
pub const AUTH_FAILURE_DELAY: Duration = Duration::from_millis(500);

/// Largest form body accepted.
const MAX_BODY_BYTES: usize = 1 << 20;

/// Settings of streaming, transcoding and cover art.
#[derive(Clone, Debug, Default)]
pub struct MediaConfig {
    /// The `ffmpeg` binary, if one is available.
    pub ffmpeg: Option<PathBuf>,
    pub transcode_format: String,
    pub transcode_bitrate_kbps: u32,
    pub cover_cache_dir: PathBuf,
}

/// Where plays are reported. Does nothing until the scrobbling task replaces it.
#[derive(Clone, Debug, Default)]
pub struct ScrobbleSink {}

/// What every handler needs.
#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub secrets: Arc<SecretBox>,
    pub search: Arc<dyn SearchIndex>,
    pub scans: ScanController,
    pub media: MediaConfig,
    pub scrobbles: ScrobbleSink,
    pub ignored_articles: Vec<String>,
}

/// The parameters of a request, from the query string and a form body. A name that
/// repeats (`id=a&id=b`) is a list.
#[derive(Clone, Debug, Default)]
pub struct Params(Vec<(String, String)>);

impl Params {
    pub fn parse(raw: &[u8]) -> Params {
        Params(
            form_urlencoded::parse(raw)
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect(),
        )
    }

    fn extend(&mut self, other: Params) {
        self.0.extend(other.0);
    }

    /// The first value of `name`.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Every value of `name`, in order.
    pub fn all(&self, name: &str) -> Vec<&str> {
        self.0
            .iter()
            .filter(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    /// The first value of `name`, or error 10.
    pub fn require(&self, name: &str) -> Result<&str, ApiError> {
        self.get(name)
            .ok_or_else(|| ApiError::missing_parameter(name))
    }
}

/// The routes `/rest/{method}` and `/rest/{method}.view`, for GET and POST.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/rest/{method}", get(handle).post(handle))
        .with_state(state)
}

async fn handle(
    State(state): State<AppState>,
    Path(method): Path<String>,
    request: Request<Body>,
) -> Response {
    let started = Instant::now();
    let params = read_params(request).await;
    let format = Format::from_param(params.get("f"));
    let method = method.strip_suffix(".view").unwrap_or(&method);
    let result = if params.get("f") == Some("jsonp") {
        Err(ApiError::generic("jsonp is not supported"))
    } else {
        dispatch(&state, method, &params).await
    };
    if let Err(e) = &result
        && e.is_auth_failure()
    {
        // Held here, in this request only: no lock is taken and no thread is blocked.
        tokio::time::sleep(AUTH_FAILURE_DELAY).await;
    }
    response::render(format, result, started.elapsed())
}

/// Query string plus, for a POST with a form body, the body.
async fn read_params(request: Request<Body>) -> Params {
    let (parts, body) = request.into_parts();
    let mut params = Params::parse(parts.uri.query().unwrap_or_default().as_bytes());
    let is_form = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.trim_start()
                .to_ascii_lowercase()
                .starts_with("application/x-www-form-urlencoded")
        });
    if parts.method == Method::POST
        && is_form
        && let Ok(bytes) = to_bytes(body, MAX_BODY_BYTES).await
    {
        params.extend(Params::parse(&bytes));
    }
    params
}

async fn dispatch(state: &AppState, method: &str, params: &Params) -> Result<Payload, ApiError> {
    let Caller(access) = authenticate(state, params).await?;
    match method {
        "ping" => Ok(system::ping()),
        "getLicense" => Ok(system::get_license()),
        "getOpenSubsonicExtensions" => Ok(system::get_open_subsonic_extensions()),
        "getMusicFolders" => system::get_music_folders(state, &access).await,
        "getUser" => system::get_user(state, &access, params).await,
        "getScanStatus" => Ok(system::get_scan_status(state, &access)),
        "startScan" => system::start_scan(state, &access),
        _ => Err(ApiError::not_implemented()),
    }
}
