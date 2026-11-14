//! `stream`, `download` and `getCoverArt`: raw bodies instead of a Subsonic node.
//!
//! Errors stay Subsonic errors: 70 for a track or cover that does not exist or that the
//! caller cannot reach, 50 for a download without the role. Music files and folders are
//! only read.

use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

use armonika_core::access::Access;
use armonika_core::ids::{ItemRef, LibraryAlbumId, TrackId};
use armonika_core::libraries;
use armonika_core::model::AudioFormat;
use armonika_core::paths::rel_path_from_db;
use armonika_core::tracks::{self, Scope, TrackView};
use armonika_media::cover;
use armonika_media::transcode::{self, TranscodePlan};
use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode, header};
use axum::response::Response;
use tower::ServiceExt;
use tower_http::services::ServeFile;

use crate::error::ApiError;
use crate::{AppState, Outcome, Params};

/// Largest `size` of a cover that is honored.
const MAX_COVER_SIZE: u32 = 4096;

/// A track the caller can see, with the file it lives in.
struct Located {
    track: TrackView,
    path: PathBuf,
}

/// Looks `id` up with the visibility of the caller and finds its file in the library.
async fn locate(state: &AppState, access: &Access, id: &str) -> Result<Located, ApiError> {
    let id: TrackId = id.parse().map_err(|_| ApiError::not_found())?;
    let libs = access.visible(None);
    let user = access.user.clone();
    let articles = state.ignored_articles.clone();
    let (track, root) = state
        .db
        .read(move |c| {
            let scope = Scope {
                libs: &libs,
                user: &user,
                articles: &articles,
            };
            let Some(track) = tracks::track(c, &scope, &id)? else {
                return Ok(None);
            };
            let root = libraries::get(c, track.library)?.map(|l| l.root);
            Ok(root.map(|root| (track, root)))
        })
        .await?
        .ok_or_else(ApiError::not_found)?;
    let path = root.join(rel_path_from_db(&track.path));
    Ok(Located { track, path })
}

pub(crate) async fn stream(
    state: &AppState,
    access: &Access,
    params: &Params,
    headers: &HeaderMap,
) -> Result<Outcome, ApiError> {
    let located = locate(state, access, params.require("id")?).await?;
    let max_kbps = number(params, "maxBitRate")?;
    let offset = number(params, "timeOffset")?;
    let plan =
        AudioFormat::from_extension(&located.track.suffix).map_or(TranscodePlan::Raw, |source| {
            transcode::plan(
                source,
                located.track.bitrate_kbps,
                params.get("format"),
                max_kbps,
                (
                    &state.media.transcode_format,
                    state.media.transcode_bitrate_kbps,
                ),
            )
        });
    if let TranscodePlan::Encode { format, .. } = plan {
        if !is_file(&located.path).await {
            return Err(vanished(state, &located));
        }
        let ffmpeg = state
            .media
            .ffmpeg
            .as_deref()
            .unwrap_or_else(|| Path::new("ffmpeg"));
        match transcode::spawn(ffmpeg, &located.path, plan, offset) {
            Ok(stream) => {
                let mut response = Response::new(Body::from_stream(stream));
                response.headers_mut().insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static(format.content_type()),
                );
                return Ok(Outcome::Raw(response));
            }
            Err(err) => {
                if !WARNED_NO_FFMPEG.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        error = %err,
                        "cannot run ffmpeg, tracks are streamed as they are"
                    );
                }
            }
        }
    }
    serve(state, &located, headers, false)
        .await
        .map(Outcome::Raw)
}

/// Set once the missing ffmpeg has been logged.
static WARNED_NO_FFMPEG: AtomicBool = AtomicBool::new(false);

/// The number in parameter `name`, error 0 when there is a value that is not one.
fn number(params: &Params, name: &str) -> Result<Option<u32>, ApiError> {
    match params.get(name) {
        None => Ok(None),
        Some(raw) => raw
            .trim()
            .parse::<u32>()
            .map(Some)
            .map_err(|_| ApiError::generic(format!("invalid {name}"))),
    }
}

async fn is_file(path: &Path) -> bool {
    tokio::fs::metadata(path).await.is_ok_and(|m| m.is_file())
}

/// The track is gone from the disk: the library is scanned again so it leaves the
/// catalog, and the caller gets 70.
fn vanished(state: &AppState, located: &Located) -> ApiError {
    state.scans.request(located.track.library);
    ApiError::not_found()
}

pub(crate) async fn download(
    state: &AppState,
    access: &Access,
    params: &Params,
    headers: &HeaderMap,
) -> Result<Outcome, ApiError> {
    // Visibility first, so a track of another library is 70 whatever the role.
    let located = locate(state, access, params.require("id")?).await?;
    if !access.can_download {
        return Err(ApiError::unauthorized());
    }
    serve(state, &located, headers, true)
        .await
        .map(Outcome::Raw)
}

/// The original file with HTTP range support. A file that is gone from the disk is 70,
/// and the library is scanned again so the track leaves the catalog.
async fn serve(
    state: &AppState,
    located: &Located,
    headers: &HeaderMap,
    attachment: bool,
) -> Result<Response, ApiError> {
    if !is_file(&located.path).await {
        return Err(vanished(state, located));
    }
    let mut request = Request::builder().method("GET").uri("/");
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let request = request
        .body(Body::empty())
        .map_err(|_| ApiError::generic("bad request"))?;
    let response = match ServeFile::new(&located.path).oneshot(request).await {
        Ok(response) => response,
        Err(never) => match never {},
    };
    if response.status() == StatusCode::NOT_FOUND {
        return Err(vanished(state, located));
    }
    let mut response = response.map(Body::new);
    if response.status().is_success()
        && let Ok(value) = HeaderValue::from_str(&located.track.content_type)
    {
        response.headers_mut().insert(header::CONTENT_TYPE, value);
    }
    if attachment {
        let name = located
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("track");
        if let Ok(value) = HeaderValue::from_str(&content_disposition(name)) {
            response
                .headers_mut()
                .insert(header::CONTENT_DISPOSITION, value);
        }
    }
    Ok(response)
}

/// `attachment` with the file name: an ASCII fallback and the exact name (RFC 6266).
fn content_disposition(name: &str) -> String {
    let fallback: String = name
        .chars()
        .map(|c| {
            if c.is_ascii() && !c.is_ascii_control() && c != '"' && c != '\\' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut encoded = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            encoded.push(char::from(b));
        } else {
            let _ = write!(encoded, "%{b:02X}");
        }
    }
    format!("attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

// ------------------------------------------------------------------ cover art

/// Where the picture of a cover comes from.
#[derive(Clone, Debug)]
enum Source {
    /// An image file in the folder of the tracks.
    Folder(PathBuf),
    /// The picture embedded in a track file.
    Embedded(PathBuf),
}

impl Source {
    fn path(&self) -> &Path {
        match self {
            Source::Folder(p) | Source::Embedded(p) => p,
        }
    }
}

pub(crate) async fn get_cover_art(
    state: &AppState,
    access: &Access,
    params: &Params,
) -> Result<Outcome, ApiError> {
    let raw = params.require("id")?;
    let size = match params.get("size") {
        None => None,
        Some(raw) => match raw.trim().parse::<u32>() {
            Ok(0) => None,
            Ok(n) => Some(n.min(MAX_COVER_SIZE)),
            Err(_) => return Err(ApiError::generic("invalid size")),
        },
    };
    let item: ItemRef = raw.parse().map_err(|_| ApiError::not_found())?;
    // The id of the cover (canonical), the track to take it from, and which place comes
    // first: an album prefers the folder, a track its own picture.
    let (cover_id, located, folder_first) = match item {
        ItemRef::Track(id) => {
            let located = locate(state, access, id.as_str()).await?;
            (located.track.id.as_str().to_owned(), located, false)
        }
        ItemRef::Album(id) => {
            let (album, first) = first_track_of_album(state, access, id).await?;
            let located = locate(state, access, first.as_str()).await?;
            (album, located, true)
        }
        ItemRef::Artist(_) => return Err(ApiError::not_found()),
    };
    let cache_dir = state.media.cover_cache_dir.clone();
    let (bytes, mime) = tokio::task::spawn_blocking(move || {
        cover_bytes(&cache_dir, &cover_id, &located.path, folder_first, size)
    })
    .await
    .map_err(|_| ApiError::generic("internal error"))?
    .ok_or_else(ApiError::not_found)?;

    let mut response = Response::new(Body::from(bytes));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    Ok(Outcome::Raw(response))
}

/// The identifier of the album and of its first track, if the caller sees the album.
async fn first_track_of_album(
    state: &AppState,
    access: &Access,
    id: LibraryAlbumId,
) -> Result<(String, TrackId), ApiError> {
    let libs = access.visible(None);
    let user = access.user.clone();
    let articles = state.ignored_articles.clone();
    state
        .db
        .read(move |c| {
            let scope = Scope {
                libs: &libs,
                user: &user,
                articles: &articles,
            };
            Ok(tracks::album(c, &scope, &id)?.and_then(|(album, tracks)| {
                tracks
                    .first()
                    .map(|t| (album.id.as_str().to_owned(), t.id.clone()))
            }))
        })
        .await?
        .ok_or_else(ApiError::not_found)
}

/// Milliseconds since the epoch of the last change of `path`.
fn mtime_ms(path: &Path) -> Option<u128> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).ok()?.as_millis())
}

fn cache_name(id: &str, size: Option<u32>, mtime: u128) -> String {
    match size {
        Some(size) => format!("{id}-{size}-{mtime}.jpg"),
        None => format!("{id}-orig-{mtime}.jpg"),
    }
}

/// The bytes and content type of the cover of the track at `track_path`, from the cache
/// when it holds them.
fn cover_bytes(
    cache_dir: &Path,
    id: &str,
    track_path: &Path,
    folder_first: bool,
    size: Option<u32>,
) -> Option<(Vec<u8>, &'static str)> {
    let folder = track_path
        .parent()
        .and_then(cover::folder)
        .map(Source::Folder);
    let mut order = vec![Some(Source::Embedded(track_path.to_path_buf())), folder];
    if folder_first {
        order.reverse();
    }
    let candidates: Vec<(Source, u128)> = order
        .into_iter()
        .flatten()
        .filter_map(|s| mtime_ms(s.path()).map(|m| (s, m)))
        .collect();

    // A cover already made for one of the places, in order of preference.
    for (_, mtime) in &candidates {
        if let Ok(bytes) = fs::read(cache_dir.join(cache_name(id, size, *mtime))) {
            return with_type(bytes);
        }
    }
    for (source, mtime) in &candidates {
        let Some(original) = read_source(source) else {
            continue;
        };
        let bytes = match size {
            Some(size) => match cover::resize(&original, size) {
                Ok(bytes) => bytes,
                Err(err) => {
                    tracing::warn!(error = %err, "cannot resize a cover");
                    continue;
                }
            },
            None => original,
        };
        store(cache_dir, &cache_name(id, size, *mtime), &bytes);
        return with_type(bytes);
    }
    None
}

/// The picture of `source`, `None` if there is none or it cannot be read.
fn read_source(source: &Source) -> Option<Vec<u8>> {
    match source {
        Source::Folder(path) => fs::read(path).ok(),
        Source::Embedded(path) => match cover::embedded(path) {
            Ok(image) => image.map(|i| i.bytes),
            Err(err) => {
                tracing::warn!(error = %err, "cannot read the embedded cover");
                None
            }
        },
    }
}

/// Writes a cache file through a temporary name, so a reader never sees half of it. A
/// failure only costs the cache.
fn store(cache_dir: &Path, name: &str, bytes: &[u8]) {
    let result = fs::create_dir_all(cache_dir).and_then(|()| {
        let tmp = cache_dir.join(format!("{name}.tmp"));
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, cache_dir.join(name))
    });
    if let Err(err) = result {
        tracing::warn!(error = %err, "cannot write the cover cache");
    }
}

/// `bytes` with the content type their first bytes show.
fn with_type(bytes: Vec<u8>) -> Option<(Vec<u8>, &'static str)> {
    let mime = if bytes.starts_with(&[0xFF, 0xD8]) {
        "image/jpeg"
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "application/octet-stream"
    };
    Some((bytes, mime))
}
