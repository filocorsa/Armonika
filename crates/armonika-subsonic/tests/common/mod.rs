#![allow(dead_code)]

use std::fmt::Write;
use std::path::PathBuf;
use std::sync::Arc;

use armonika_core::access::{self, Permission};
use armonika_core::db::Db;
use armonika_core::ids::LibraryId;
use armonika_core::model::{AudioFormat, FileFacts, TagFacts};
use armonika_core::secret::SecretBox;
use armonika_core::users::{self, NewUser};
use armonika_core::{libraries, track_files};
use armonika_scanner::ScanOptions;
use armonika_scanner::controller::ScanController;
use armonika_search::{SearchIndex, TantivyIndex};
use armonika_subsonic::{AppState, MediaConfig, ScrobbleSink, router};
use armonika_testdata::AudioFormat as TestFormat;
use armonika_testdata::fixture::{FixtureTags, tagged_file};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Method, Request, StatusCode};
use md5::{Digest, Md5};
use tower::ServiceExt;

pub const ADMIN_PASSWORD: &str = "admin-password";
pub const ANN_PASSWORD: &str = "ann-password";

/// A server with the users `admin` (admin) and `ann` (listen on library A, no download),
/// the libraries A (3 tracks) and B, and an API key of `ann`.
pub struct App {
    pub music_a: PathBuf,
    pub music_b: PathBuf,
    pub router: Router,
    pub db: Db,
    pub search: Arc<dyn SearchIndex>,
    pub scans: ScanController,
    pub cache_dir: PathBuf,
    pub lib_a: LibraryId,
    pub lib_b: LibraryId,
    pub ann_key: String,
    _dir: tempfile::TempDir,
}

pub async fn app() -> App {
    build(true, MediaConfig::default()).await
}

/// Same as [`app`] but without the three tracks of library A: music comes from files
/// written with [`App::put`] and read with [`App::scan`].
pub async fn app_empty() -> App {
    build(false, MediaConfig::default()).await
}

/// Same as [`app_empty`] with the given transcoding settings (the cover cache folder is
/// set here).
pub async fn app_with_media(media: MediaConfig) -> App {
    build(false, media).await
}

async fn build(with_tracks: bool, media: MediaConfig) -> App {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(&dir.path().join("armonika.db")).unwrap();
    let secrets = SecretBox::from_key([9; 32]);
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    for name in ["a", "b"] {
        std::fs::create_dir_all(dir.path().join(name)).unwrap();
    }

    let s = secrets.clone();
    let root = dir.path().to_owned();
    let (lib_a, lib_b, ann_key) = db
        .write(move |c| {
            let lib_a = libraries::create(c, "A", &root.join("a"), &data_dir)?;
            let lib_b = libraries::create(c, "B", &root.join("b"), &data_dir)?;
            let user = |name: &str, password: &str, is_admin, can_download| NewUser {
                name: name.to_owned(),
                password: password.to_owned(),
                is_admin,
                can_download,
            };
            users::create(c, &s, user("admin", ADMIN_PASSWORD, true, true))?;
            let ann = users::create(c, &s, user("ann", ANN_PASSWORD, false, false))?;
            access::grant(c, &ann, lib_a, Permission::Listen)?;
            let key = users::create_api_key(c, &ann, "test")?;
            let tx = c.transaction()?;
            for n in 1..=if with_tracks { 3 } else { 0 } {
                let facts = FileFacts {
                    size: 1,
                    mtime_ms: 1,
                    format: AudioFormat::Flac,
                    duration_ms: Some(1000),
                    bitrate_kbps: None,
                    sample_rate: None,
                    channels: None,
                    bit_depth: None,
                    tags: TagFacts {
                        title: Some(format!("Track {n}")),
                        ..TagFacts::default()
                    },
                };
                track_files::insert_track(&tx, lib_a, &format!("t{n}.flac"), &facts, 1)?;
            }
            tx.commit()?;
            Ok((lib_a, lib_b, key))
        })
        .await
        .unwrap();

    let search: Arc<dyn SearchIndex> =
        Arc::new(TantivyIndex::open(&dir.path().join("search")).unwrap());
    let (changes, _rx) = tokio::sync::mpsc::unbounded_channel();
    let scans = ScanController::new(db.clone(), ScanOptions::default(), changes);
    let cache_dir = dir.path().join("data").join("cache").join("covers");
    let state = AppState {
        db: db.clone(),
        secrets: Arc::new(secrets),
        search: search.clone(),
        scans: scans.clone(),
        media: MediaConfig {
            cover_cache_dir: cache_dir.clone(),
            ..media
        },
        scrobbles: ScrobbleSink::default(),
        ignored_articles: "The El La Los Las Le Les Il Lo Gli I"
            .split_whitespace()
            .map(String::from)
            .collect(),
    };
    App {
        music_a: dir.path().join("a"),
        music_b: dir.path().join("b"),
        router: router(state),
        db,
        search,
        scans,
        cache_dir,
        lib_a,
        lib_b,
        ann_key,
        _dir: dir,
    }
}

impl App {
    /// Writes an mp3 with `tags` at `rel` in the music folder `root` (`music_a` or
    /// `music_b`).
    pub fn put(&self, root: &std::path::Path, rel: &str, tags: &FixtureTags) {
        let path = root.join(rel);
        let dir = path.parent().unwrap();
        std::fs::create_dir_all(dir).unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        tagged_file(dir, name, TestFormat::Mp3, tags);
    }

    /// Scans both libraries.
    pub fn scan(&self) {
        for lib in [self.lib_a, self.lib_b] {
            armonika_scanner::scan_library(
                &self.db,
                lib,
                armonika_scanner::ScanOptions {
                    threads: 2,
                    batch: 500,
                    allow_empty: false,
                },
                &|_| {},
                &|_| {},
            )
            .unwrap();
        }
    }

    /// Rebuilds the search index from the database (the scanner does not feed it here).
    pub async fn index(&self) {
        armonika_search::updater::rebuild(self.db.clone(), self.search.clone())
            .await
            .unwrap();
    }

    /// Runs `sql` on the writer connection.
    pub async fn exec(&self, sql: &'static str) {
        self.db
            .write(move |c| Ok(c.execute_batch(sql)?))
            .await
            .unwrap();
    }
}

pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: String,
    pub bytes: Vec<u8>,
}

impl Reply {
    /// The body as JSON, which must be a `subsonic-response`; returns its content.
    pub fn json(&self) -> serde_json::Value {
        let v: serde_json::Value = serde_json::from_str(&self.body).unwrap();
        v["subsonic-response"].clone()
    }
}

pub async fn get(app: &App, uri: &str) -> Reply {
    send(app, Method::GET, uri, None, &[]).await
}

/// A GET with extra request headers.
pub async fn get_with(app: &App, uri: &str, headers: &[(&str, &str)]) -> Reply {
    send(app, Method::GET, uri, None, headers).await
}

pub async fn post_form(app: &App, uri: &str, form: &str) -> Reply {
    send(app, Method::POST, uri, Some(form), &[]).await
}

async fn send(
    app: &App,
    method: Method,
    uri: &str,
    form: Option<&str>,
    headers: &[(&str, &str)],
) -> Reply {
    let mut req = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    if form.is_some() {
        req = req.header("content-type", "application/x-www-form-urlencoded");
    }
    let req = req
        .body(Body::from(form.unwrap_or_default().to_owned()))
        .unwrap();
    let resp = app.router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    Reply {
        status,
        headers,
        body: String::from_utf8_lossy(&bytes).into_owned(),
        bytes: bytes.to_vec(),
    }
}

/// `t` for `password` and `salt`.
pub fn token(password: &str, salt: &str) -> String {
    let digest = Md5::digest(format!("{password}{salt}"));
    digest.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Query of a token login of `user`, with the JSON format.
pub fn login(user: &str, password: &str) -> String {
    format!(
        "u={user}&t={}&s=salty&v=1.16.1&c=test&f=json",
        token(password, "salty")
    )
}
