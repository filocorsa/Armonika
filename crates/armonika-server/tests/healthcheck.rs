//! `armonika healthcheck`: alive means the server answers `/rest/ping`.

use std::io::{Read, Write};
use std::net::TcpListener;

/// A one-shot HTTP server that answers every request with `status` and `body`.
fn fake_server(status: &'static str, body: &'static str) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            let reply = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    port
}

fn run(port: u16) -> std::process::Output {
    assert_cmd::Command::cargo_bin("armonika")
        .unwrap()
        .arg("healthcheck")
        .env_remove("ARMONIKA_CONFIG")
        .env("ARMONIKA_SERVER__BIND", format!("0.0.0.0:{port}"))
        .output()
        .unwrap()
}

#[test]
fn ok_response_is_alive() {
    let port = fake_server(
        "200 OK",
        r#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#,
    );
    assert!(run(port).status.success());
}

#[test]
fn error_10_response_is_alive() {
    let port = fake_server(
        "200 OK",
        r#"{"subsonic-response":{"status":"failed","error":{"code":10,"message":"missing"}}}"#,
    );
    assert!(run(port).status.success());
}

#[test]
fn other_error_is_not_alive() {
    let port = fake_server(
        "200 OK",
        r#"{"subsonic-response":{"status":"failed","error":{"code":0,"message":"boom"}}}"#,
    );
    assert!(!run(port).status.success());
}

#[test]
fn http_error_is_not_alive() {
    let port = fake_server("500 Internal Server Error", "oops");
    assert!(!run(port).status.success());
}

#[test]
fn nothing_listening_is_not_alive() {
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    assert!(!run(port).status.success());
}

/// Kills the server process even when an assertion fails.
struct KillOnDrop(std::process::Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn real_server_is_alive_without_credentials() {
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let dir = tempfile::tempdir().unwrap();
    let child = std::process::Command::new(assert_cmd::cargo::cargo_bin("armonika"))
        .arg("serve")
        .env_remove("ARMONIKA_CONFIG")
        .env_remove("ARMONIKA_SECRET_KEY")
        .env("ARMONIKA_SERVER__BIND", format!("127.0.0.1:{port}"))
        .env("ARMONIKA_DATA_DIR", dir.path().join("data"))
        .env("ARMONIKA_SCAN__WATCH", "false")
        .env("ARMONIKA_SCAN__INTERVAL", "0s")
        .current_dir(dir.path())
        .spawn()
        .unwrap();
    let _guard = KillOnDrop(child);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if run(port).status.success() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "healthcheck never succeeded"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
