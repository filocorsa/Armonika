//! The OpenSubsonic API of the Armonika music server, served under `/rest`.

mod annotate;
mod auth;
mod browse;
mod error;
mod media;
mod playlists;
mod response;
mod search;
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
use axum::http::{HeaderMap, Method, Request, header};
use axum::response::Response;
use axum::routing::get;

pub use armonika_scrobble::ScrobbleSink;
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

/// What a handler answers: a node of the Subsonic response, or a raw body (audio, an
/// image) that replaces the response. Errors are always Subsonic errors.
pub(crate) enum Outcome {
    Payload(Payload),
    Raw(Response),
}

impl From<Payload> for Outcome {
    fn from(payload: Payload) -> Outcome {
        Outcome::Payload(payload)
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
    let headers = request.headers().clone();
    let params = read_params(request).await;
    let format = Format::from_param(params.get("f"));
    let method = method.strip_suffix(".view").unwrap_or(&method);
    let result = if params.get("f") == Some("jsonp") {
        Err(ApiError::generic("jsonp is not supported"))
    } else {
        dispatch(&state, method, &params, &headers).await
    };
    if let Err(e) = &result
        && e.is_auth_failure()
    {
        // Held here, in this request only: no lock is taken and no thread is blocked.
        tokio::time::sleep(AUTH_FAILURE_DELAY).await;
    }
    match result {
        Ok(Outcome::Raw(response)) => response,
        Ok(Outcome::Payload(payload)) => response::render(format, Ok(payload), started.elapsed()),
        Err(e) => response::render(format, Err(e), started.elapsed()),
    }
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

async fn dispatch(
    state: &AppState,
    method: &str,
    params: &Params,
    headers: &HeaderMap,
) -> Result<Outcome, ApiError> {
    let Caller(access) = authenticate(state, params).await?;
    match method {
        "stream" => return media::stream(state, &access, params, headers).await,
        "download" => return media::download(state, &access, params, headers).await,
        "getCoverArt" => return media::get_cover_art(state, &access, params).await,
        _ => {}
    }
    dispatch_payload(state, method, &access, params)
        .await
        .map(Outcome::Payload)
}

async fn dispatch_payload(
    state: &AppState,
    method: &str,
    access: &armonika_core::access::Access,
    params: &Params,
) -> Result<Payload, ApiError> {
    match method {
        "ping" => Ok(system::ping()),
        "getLicense" => Ok(system::get_license()),
        "getOpenSubsonicExtensions" => Ok(system::get_open_subsonic_extensions()),
        "getMusicFolders" => system::get_music_folders(state, access).await,
        "getUser" => system::get_user(state, access, params).await,
        "getScanStatus" => Ok(system::get_scan_status(state, access)),
        "startScan" => system::start_scan(state, access),
        "getArtists" => browse::get_artists(state, access, params).await,
        "getIndexes" => browse::get_indexes(state, access, params).await,
        "getArtist" => browse::get_artist(state, access, params).await,
        "getAlbum" => browse::get_album(state, access, params).await,
        "getSong" => browse::get_song(state, access, params).await,
        "getMusicDirectory" => browse::get_music_directory(state, access, params).await,
        "getGenres" => browse::get_genres(state, access, params).await,
        "getAlbumList2" => browse::get_album_list2(state, access, params).await,
        "getAlbumList" => browse::get_album_list(state, access, params).await,
        "getRandomSongs" => browse::get_random_songs(state, access, params).await,
        "getSongsByGenre" => browse::get_songs_by_genre(state, access, params).await,
        "search3" => search::search(state, access, params, "searchResult3").await,
        "search2" => search::search(state, access, params, "searchResult2").await,
        "star" => annotate::star(state, access, params).await,
        "unstar" => annotate::unstar(state, access, params).await,
        "setRating" => annotate::set_rating(state, access, params).await,
        "scrobble" => annotate::scrobble(state, access, params).await,
        "getStarred2" => annotate::get_starred(state, access, "starred2").await,
        "getStarred" => annotate::get_starred(state, access, "starred").await,
        "getNowPlaying" => annotate::get_now_playing(state, access).await,
        "getPlaylists" => playlists::get_playlists(state, access).await,
        "getPlaylist" => playlists::get_playlist(state, access, params).await,
        "createPlaylist" => playlists::create_playlist(state, access, params).await,
        "updatePlaylist" => playlists::update_playlist(state, access, params).await,
        "deletePlaylist" => playlists::delete_playlist(state, access, params).await,
        "savePlayQueue" => playlists::save_play_queue(state, access, params).await,
        "getPlayQueue" => playlists::get_play_queue(state, access).await,
        "getArtistInfo2" => browse::get_artist_info2(params),
        "getAlbumInfo2" => browse::get_album_info2(params),
        "getTopSongs" => Ok(browse::get_top_songs()),
        "getSimilarSongs2" => Ok(browse::get_similar_songs2()),
        _ => Err(ApiError::not_implemented()),
    }
}
