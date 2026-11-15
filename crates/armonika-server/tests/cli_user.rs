use std::path::Path;

/// Runs `armonika <args>` with a fresh environment pointing at `dir`, feeding `stdin`.
/// Returns (success, stdout, stderr).
fn armonika(dir: &Path, args: &[&str], stdin: &str) -> (bool, String, String) {
    let out = assert_cmd::Command::cargo_bin("armonika")
        .unwrap()
        .args(args)
        .env_remove("ARMONIKA_CONFIG")
        .env_remove("ARMONIKA_SECRET_KEY")
        .env_remove("ARMONIKA_ADMIN_USER")
        .env_remove("ARMONIKA_ADMIN_PASSWORD")
        .env("ARMONIKA_DATA_DIR", dir.join("data"))
        .current_dir(dir)
        .write_stdin(stdin)
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn user_create_then_list() {
    let dir = tempfile::tempdir().unwrap();
    let (ok, out, err) = armonika(
        dir.path(),
        &["user", "create", "alice", "--admin", "--password-stdin"],
        "correct horse\n",
    );
    assert!(ok, "{out}{err}");
    let (ok, out, err) = armonika(
        dir.path(),
        &["user", "create", "bob", "--no-download", "--password-stdin"],
        "battery staple\r\n",
    );
    assert!(ok, "{out}{err}");

    let (ok, out, err) = armonika(dir.path(), &["user", "list"], "");
    assert!(ok, "{out}{err}");
    let alice = out.lines().find(|l| l.contains("alice")).expect(&out);
    assert!(alice.contains("admin"), "{out}");
    let bob = out.lines().find(|l| l.contains("bob")).expect(&out);
    assert!(
        !bob.contains("admin") && bob.contains("no-download"),
        "{out}"
    );

    // The trailing newline (and CR) are not part of the password.
    let (ok, out, _) = armonika(dir.path(), &["user", "show-subsonic-password", "bob"], "");
    assert!(ok);
    assert_eq!(out.trim_end_matches(['\r', '\n']), "battery staple");

    let (ok, _, err) = armonika(dir.path(), &["user", "delete", "alice"], "");
    assert!(!ok, "deleting the last admin must fail");
    assert!(err.contains("last admin"), "{err}");
    let (ok, out, err) = armonika(dir.path(), &["user", "delete", "bob"], "");
    assert!(ok, "{out}{err}");
    let (_, out, _) = armonika(dir.path(), &["user", "list"], "");
    assert!(!out.contains("bob"), "{out}");
}

#[test]
fn password_never_in_output() {
    let dir = tempfile::tempdir().unwrap();
    let secrets = [
        "first-secret-pw",
        "second-secret-pw",
        "subsonic-secret-pw",
        "sh0rt",
        "x-secret-pw",
    ];
    // (arguments, stdin, expected to succeed)
    let runs: Vec<(Vec<&str>, &str, bool)> = vec![
        (
            vec!["user", "create", "carol", "--password-stdin"],
            "first-secret-pw\n",
            true,
        ),
        (
            vec!["user", "set-password", "carol", "--password-stdin"],
            "second-secret-pw\n",
            true,
        ),
        (
            vec!["user", "set-subsonic-password", "carol", "--password-stdin"],
            "subsonic-secret-pw\n",
            true,
        ),
        (vec!["user", "list"], "", true),
        (
            vec![
                "user",
                "set",
                "carol",
                "--download",
                "false",
                "--share-now-playing",
                "true",
            ],
            "",
            true,
        ),
        // Too short: refused.
        (
            vec!["user", "create", "dave", "--password-stdin"],
            "sh0rt\n",
            false,
        ),
        // No password can be given as an argument.
        (
            vec!["user", "create", "erin", "--password", "x-secret-pw"],
            "",
            false,
        ),
    ];
    for (args, stdin, expect_ok) in &runs {
        let (ok, out, err) = armonika(dir.path(), args, stdin);
        assert_eq!(ok, *expect_ok, "{args:?}: {out}{err}");
        for s in secrets {
            assert!(!out.contains(s), "{args:?} printed a password: {out}");
            assert!(!err.contains(s), "{args:?} printed a password: {err}");
        }
    }
    let (_, out, _) = armonika(dir.path(), &["user", "list"], "");
    assert!(!out.contains("dave") && !out.contains("erin"), "{out}");
    assert!(
        out.contains("no-download") && out.contains("share-now-playing"),
        "{out}"
    );

    // An API key is printed once, at creation.
    let (ok, key, err) = armonika(
        dir.path(),
        &["user", "api-key", "create", "carol", "phone"],
        "",
    );
    assert!(ok, "{key}{err}");
    assert_eq!(key.trim().len(), 43, "{key}");
    let (ok, out, err) = armonika(
        dir.path(),
        &["user", "api-key", "revoke", "carol", "phone"],
        "",
    );
    assert!(ok, "{out}{err}");
    assert!(!out.contains(key.trim()));
}

#[test]
fn a_changed_secret_key_stops_the_command_and_names_its_source() {
    let dir = tempfile::tempdir().unwrap();
    // Creates the data directory, its secret.key and the canary.
    let (ok, out, err) = armonika(
        dir.path(),
        &["user", "create", "alice", "--admin", "--password-stdin"],
        "correct horse\n",
    );
    assert!(ok, "{out}{err}");
    // Same data directory, another key from the environment.
    let wrong = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    let out = assert_cmd::Command::cargo_bin("armonika")
        .unwrap()
        .args(["user", "list"])
        .env_remove("ARMONIKA_CONFIG")
        .env("ARMONIKA_SECRET_KEY", wrong)
        .env("ARMONIKA_DATA_DIR", dir.path().join("data"))
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains("ARMONIKA_SECRET_KEY"), "{err}");
    assert!(!err.contains(wrong), "{err}");
}

/// Serves `answers` (status, body) one connection each, on a free local port, and
/// returns the base URL. Stands in for ListenBrainz.
fn fake_listenbrainz(answers: Vec<(u16, &'static str)>) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for (status, body) in answers {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0; 4096];
            let _ = stream.read(&mut buf);
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    url
}

fn armonika_lb(dir: &Path, url: &str, args: &[&str], stdin: &str) -> (bool, String, String) {
    let out = assert_cmd::Command::cargo_bin("armonika")
        .unwrap()
        .args(args)
        .env_remove("ARMONIKA_CONFIG")
        .env_remove("ARMONIKA_SECRET_KEY")
        .env("ARMONIKA_DATA_DIR", dir.join("data"))
        .env("ARMONIKA_LISTENBRAINZ__URL", url)
        .current_dir(dir)
        .write_stdin(stdin)
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn listenbrainz_link_validates_and_stores_the_token_encrypted() {
    let dir = tempfile::tempdir().unwrap();
    let (ok, out, err) = armonika(
        dir.path(),
        &["user", "create", "alice", "--password-stdin"],
        "correct horse\n",
    );
    assert!(ok, "{out}{err}");

    let url = fake_listenbrainz(vec![
        (200, r#"{"code":200,"valid":false}"#),
        (200, r#"{"code":200,"valid":true,"user_name":"alicelb"}"#),
    ]);
    let args = ["user", "listenbrainz", "alice", "--token-stdin"];
    let (ok, _, err) = armonika_lb(dir.path(), &url, &args, "bad-token\n");
    assert!(!ok && err.contains("does not accept"), "{err}");
    assert!(!err.contains("bad-token"), "{err}");

    let (ok, out, err) = armonika_lb(dir.path(), &url, &args, "good-token-123\n");
    assert!(ok, "{out}{err}");
    assert!(
        out.contains("alicelb") && !out.contains("good-token-123"),
        "{out}"
    );

    let db = std::fs::read(dir.path().join("data").join("armonika.db")).unwrap();
    let wal = std::fs::read(dir.path().join("data").join("armonika.db-wal")).unwrap_or_default();
    let needle = b"good-token-123";
    assert!(
        !db.windows(needle.len()).any(|w| w == needle)
            && !wal.windows(needle.len()).any(|w| w == needle),
        "the token is stored in clear"
    );

    let (ok, out, err) = armonika(
        dir.path(),
        &["user", "scrobble-unlink", "alice", "listenbrainz"],
        "",
    );
    assert!(ok && out.contains("unlinked"), "{out}{err}");
    let (ok, out, _) = armonika(
        dir.path(),
        &["user", "scrobble-unlink", "alice", "listenbrainz"],
        "",
    );
    assert!(ok && out.contains("was not linked"), "{out}");
}
