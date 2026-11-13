#[test]
fn version_flag_prints_name_and_version() {
    let out = assert_cmd::Command::cargo_bin("armonika")
        .unwrap()
        .arg("--version")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).unwrap().trim(),
        format!("armonika {}", armonika_core::VERSION)
    );
}

#[test]
fn no_subcommand_prints_help() {
    let out = assert_cmd::Command::cargo_bin("armonika")
        .unwrap()
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("Usage:"), "{stdout}");
    assert!(stdout.contains("serve"), "{stdout}");
}

fn get_root(port: u16) -> Option<(u16, String)> {
    let mut resp = ureq::get(format!("http://127.0.0.1:{port}/"))
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .ok()?;
    let status = resp.status().as_u16();
    Some((status, resp.body_mut().read_to_string().ok()?))
}

/// Kills the server process even when an assertion fails.
struct KillOnDrop(std::process::Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Starts `armonika serve` on a free port with a fresh data directory and
/// returns the first answer to `GET /`, waiting up to 10 s.
fn serve_and_get_root(extra_env: &[(&str, &str)]) -> (u16, String) {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let mut cmd = std::process::Command::new(assert_cmd::cargo::cargo_bin("armonika"));
    cmd.arg("serve")
        .env_remove("ARMONIKA_CONFIG")
        .env("ARMONIKA_SERVER__BIND", format!("127.0.0.1:{port}"))
        .env("ARMONIKA_DATA_DIR", &data)
        .current_dir(dir.path());
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let child = KillOnDrop(cmd.spawn().unwrap());

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut answer = None;
    while std::time::Instant::now() < deadline {
        if let Some(a) = get_root(port) {
            answer = Some(a);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    drop(child);

    let (status, body) = answer.expect("server did not answer within 10 s");
    assert!(data.is_dir(), "data_dir was not created");
    (status, body)
}

#[test]
fn serve_answers_on_bind_address() {
    let (status, body) = serve_and_get_root(&[]);
    assert_eq!(status, 200);
    assert_eq!(body, format!("Armonika {}", armonika_core::VERSION));
}

#[test]
fn serve_without_web_answers_404() {
    let (status, _) = serve_and_get_root(&[("ARMONIKA_WEB__ENABLED", "false")]);
    assert_eq!(status, 404);
}
