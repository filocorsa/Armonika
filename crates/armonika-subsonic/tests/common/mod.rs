#![allow(dead_code)]

use std::fmt::Write;
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
    pub router: Router,
    pub db: Db,
    pub lib_a: LibraryId,
    pub lib_b: LibraryId,
    pub ann_key: String,
    _dir: tempfile::TempDir,
}

pub async fn app() -> App {
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
            for n in 1..=3 {
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
    let state = AppState {
        db: db.clone(),
        secrets: Arc::new(secrets),
        search,
        scans,
        media: MediaConfig::default(),
        scrobbles: ScrobbleSink::default(),
        ignored_articles: Vec::new(),
    };
    App {
        router: router(state),
        db,
        lib_a,
        lib_b,
        ann_key,
        _dir: dir,
    }
}

pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: String,
}

impl Reply {
    /// The body as JSON, which must be a `subsonic-response`; returns its content.
    pub fn json(&self) -> serde_json::Value {
        let v: serde_json::Value = serde_json::from_str(&self.body).unwrap();
        v["subsonic-response"].clone()
    }
}

pub async fn get(app: &App, uri: &str) -> Reply {
    send(app, Method::GET, uri, None).await
}

pub async fn post_form(app: &App, uri: &str, form: &str) -> Reply {
    send(app, Method::POST, uri, Some(form)).await
}

async fn send(app: &App, method: Method, uri: &str, form: Option<&str>) -> Reply {
    let mut req = Request::builder().method(method).uri(uri);
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
        body: String::from_utf8(bytes.to_vec()).unwrap(),
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
