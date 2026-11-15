//! End to end: a real `armonika serve` process over a fixture library.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use armonika_testdata::AudioFormat;
use armonika_testdata::fixture::{FixtureTags, tagged_file};

const PASSWORD: &str = "e2e-password";

/// Kills the server process even when an assertion fails.
struct Server {
    child: std::process::Child,
    port: u16,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Env {
    dir: tempfile::TempDir,
    music: PathBuf,
}

impl Env {
    /// A temporary directory with a library of one tagged mp3 file.
    fn new() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let music = dir.path().join("music");
        std::fs::create_dir_all(&music).unwrap();
        let tags = FixtureTags {
            title: Some("Zebra Crossing".to_owned()),
            artist_credit: Some("Quartz Band".to_owned()),
            artists: vec!["Quartz Band".to_owned()],
            album_title: Some("Stripes".to_owned()),
            ..FixtureTags::default()
        };
        tagged_file(&music, "zebra.mp3", AudioFormat::Mp3, &tags);
        let config = format!(
            "[[library]]\nname = \"Main\"\npath = '{}'\n",
            music.display()
        );
        std::fs::write(dir.path().join("armonika.toml"), config).unwrap();
        Env { dir, music }
    }

    fn start(&self, extra_env: &[(&str, &str)]) -> Server {
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let mut cmd = std::process::Command::new(assert_cmd::cargo::cargo_bin("armonika"));
        cmd.arg("serve")
            .env_remove("ARMONIKA_SECRET_KEY")
            .env("ARMONIKA_CONFIG", self.dir.path().join("armonika.toml"))
            .env("ARMONIKA_SERVER__BIND", format!("127.0.0.1:{port}"))
            .env("ARMONIKA_DATA_DIR", self.dir.path().join("data"))
            .env("ARMONIKA_ADMIN_USER", "admin")
            .env("ARMONIKA_ADMIN_PASSWORD", PASSWORD)
            .env("ARMONIKA_SCAN__WATCH", "false")
            .env("ARMONIKA_SCAN__INTERVAL", "0s")
            .current_dir(self.dir.path());
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        let server = Server {
            child: cmd.spawn().unwrap(),
            port,
        };
        server.wait_until_up();
        server
    }
}

fn get(url: &str) -> Option<(u16, Vec<u8>)> {
    let mut resp = ureq::get(url)
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .ok()?;
    let status = resp.status().as_u16();
    let body = resp.body_mut().read_to_vec().ok()?;
    Some((status, body))
}

/// Calls `f` until it returns `Some`, for at most 30 s.
fn poll<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

impl Server {
    fn wait_until_up(&self) {
        poll("the server to answer", || {
            get(&format!("http://127.0.0.1:{}/rest/ping?f=json", self.port))
        });
    }

    fn url(&self, method: &str, extra: &str) -> String {
        format!(
            "http://127.0.0.1:{}/rest/{method}?u=admin&p={PASSWORD}&v=1.16.1&c=e2e&f=json{extra}",
            self.port
        )
    }

    fn json(&self, method: &str, extra: &str) -> serde_json::Value {
        let (status, body) = get(&self.url(method, extra)).expect("request failed");
        assert_eq!(status, 200, "{method}");
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        v["subsonic-response"].clone()
    }

    /// The id of the track found by `query`, once the scan and the index have it.
    fn find_track(&self, query: &str) -> String {
        poll("search3 to find the track", || {
            let v = self.json("search3", &format!("&query={query}"));
            v["searchResult3"]["song"][0]["id"]
                .as_str()
                .map(str::to_owned)
        })
    }
}

fn files_equal(a: &Path, b: &[u8]) -> bool {
    std::fs::read(a).unwrap() == b
}

#[test]
fn e2e_scan_search_stream() {
    let env = Env::new();
    let server = env.start(&[]);
    // The scan is asked for at startup: wait until one has run and none is running.
    poll("the first scan to finish", || {
        let v = server.json("getScanStatus", "");
        let status = &v["scanStatus"];
        (status["scanning"] == false && status["count"].as_u64() >= Some(1)).then_some(())
    });
    let id = server.find_track("zebra");
    let (status, bytes) = get(&server.url("stream", &format!("&id={id}"))).unwrap();
    assert_eq!(status, 200);
    assert!(files_equal(&env.music.join("zebra.mp3"), &bytes));
}

#[test]
fn headless_disables_root_but_keeps_api() {
    let env = Env::new();
    let server = env.start(&[("ARMONIKA_WEB__ENABLED", "false")]);
    let (status, _) = get(&format!("http://127.0.0.1:{}/", server.port)).unwrap();
    assert_eq!(status, 404);
    let v = server.json("ping", "");
    assert_eq!(v["status"], "ok");
    server.find_track("zebra");

    let env = Env::new();
    let server = env.start(&[]);
    let (status, _) = get(&format!("http://127.0.0.1:{}/", server.port)).unwrap();
    assert_eq!(status, 200);
}

#[test]
fn restart_keeps_ids_and_stars() {
    let env = Env::new();
    let id = {
        let server = env.start(&[]);
        let id = server.find_track("zebra");
        let v = server.json("star", &format!("&id={id}"));
        assert_eq!(v["status"], "ok");
        id
    };
    let server = env.start(&[]);
    assert_eq!(server.find_track("zebra"), id);
    let v = server.json("getStarred2", "");
    assert_eq!(v["starred2"]["song"][0]["id"], id.as_str(), "{v}");
}

#[test]
fn taken_address_exits_with_an_error() {
    let env = Env::new();
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let out = std::process::Command::new(assert_cmd::cargo::cargo_bin("armonika"))
        .arg("serve")
        .env("ARMONIKA_CONFIG", env.dir.path().join("armonika.toml"))
        .env("ARMONIKA_SERVER__BIND", format!("127.0.0.1:{port}"))
        .env("ARMONIKA_DATA_DIR", env.dir.path().join("data"))
        .env("ARMONIKA_SCAN__WATCH", "false")
        .env_remove("ARMONIKA_SECRET_KEY")
        .current_dir(env.dir.path())
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("cannot bind"), "{err}");
    assert!(
        err.contains("shutdown complete")
            || String::from_utf8_lossy(&out.stdout).contains("shutdown complete")
    );
}

#[cfg(unix)]
#[test]
fn sigterm_stops_the_server_cleanly() {
    let env = Env::new();
    let mut server = env.start(&[]);
    server.find_track("zebra");
    let pid = server.child.id().to_string();
    assert!(
        std::process::Command::new("kill")
            .args(["-TERM", &pid])
            .status()
            .unwrap()
            .success()
    );
    let status = poll("the server to exit", || server.child.try_wait().unwrap());
    assert!(status.success(), "{status}");
}
