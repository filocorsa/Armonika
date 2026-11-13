use std::path::Path;

/// Runs `armonika <args>` with a fresh environment pointing at `dir`.
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

fn mkdir(dir: &Path, rel: &str) -> String {
    let path = dir.join(rel);
    std::fs::create_dir_all(&path).unwrap();
    path.to_str().unwrap().to_owned()
}

#[test]
fn library_add_list_rename_set_path() {
    let dir = tempfile::tempdir().unwrap();
    let music = mkdir(dir.path(), "music");
    let moved = mkdir(dir.path(), "moved");
    let nested = mkdir(dir.path(), "moved/inner");

    let (ok, out, err) = armonika(dir.path(), &["library", "add", "Main", &music], "");
    assert!(ok, "{out}{err}");
    let (ok, _, err) = armonika(dir.path(), &["library", "add", "Inner", &nested], "");
    assert!(ok, "{err}");
    let (ok, _, err) = armonika(dir.path(), &["library", "add", "Again", &music], "");
    assert!(!ok);
    assert!(err.contains("overlaps"), "{err}");

    let (ok, out, err) = armonika(dir.path(), &["library", "list"], "");
    assert!(ok, "{out}{err}");
    let main = out.lines().find(|l| l.contains("Main")).expect(&out);
    assert!(main.contains("music"), "{out}");

    let (ok, out, err) = armonika(dir.path(), &["library", "rename", "Main", "Home"], "");
    assert!(ok, "{out}{err}");
    let (ok, _, err) = armonika(dir.path(), &["library", "set-path", "Home", &moved], "");
    assert!(!ok, "moved contains the Inner library");
    assert!(err.contains("overlaps"), "{err}");
    let other = mkdir(dir.path(), "other");
    let (ok, out, err) = armonika(dir.path(), &["library", "set-path", "Home", &other], "");
    assert!(ok, "{out}{err}");

    let (_, out, _) = armonika(dir.path(), &["library", "list"], "");
    assert!(!out.contains("Main"), "{out}");
    let home = out.lines().find(|l| l.contains("Home")).expect(&out);
    assert!(home.contains("other") && home.starts_with('1'), "{out}");

    let (ok, _, err) = armonika(dir.path(), &["library", "rename", "Nope", "X"], "");
    assert!(!ok);
    assert!(err.contains("Nope"), "{err}");
}

#[test]
fn remove_without_yes_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let music = mkdir(dir.path(), "music");
    let song = dir.path().join("music").join("song.flac");
    std::fs::write(&song, b"x").unwrap();
    let (ok, _, err) = armonika(dir.path(), &["library", "add", "Main", &music], "");
    assert!(ok, "{err}");

    let (ok, out, err) = armonika(dir.path(), &["library", "remove", "Main"], "");
    assert!(ok, "{out}{err}");
    assert!(out.contains("0 tracks") && out.contains("--yes"), "{out}");
    let (_, out, _) = armonika(dir.path(), &["library", "list"], "");
    assert!(out.contains("Main"), "{out}");

    let (ok, out, err) = armonika(dir.path(), &["library", "remove", "Main", "--yes"], "");
    assert!(ok, "{out}{err}");
    let (_, out, _) = armonika(dir.path(), &["library", "list"], "");
    assert!(!out.contains("Main"), "{out}");
    assert!(song.is_file());
}

/// The phrase of the hint printed when a removed or renamed library is still configured.
const CONFIG_HINT: &str = "otherwise it will be created again at the next start";

#[test]
fn remove_and_rename_hint_when_library_is_configured() {
    let dir = tempfile::tempdir().unwrap();
    let music = mkdir(dir.path(), "music");
    let jazz = mkdir(dir.path(), "jazz");
    let other = mkdir(dir.path(), "other");
    std::fs::write(
        dir.path().join("armonika.toml"),
        format!(
            "[[library]]\nname = \"Main\"\npath = '{music}'\n\n\
             [[library]]\nname = \"Jazz\"\npath = '{jazz}'\n"
        ),
    )
    .unwrap();
    for (name, path) in [("Main", &music), ("Jazz", &jazz), ("Other", &other)] {
        let (ok, _, err) = armonika(dir.path(), &["library", "add", name, path], "");
        assert!(ok, "{err}");
    }

    // A configured library: renaming it prints the hint on stderr.
    let (ok, out, err) = armonika(dir.path(), &["library", "rename", "Main", "Home"], "");
    assert!(ok, "{out}{err}");
    assert!(
        err.contains(CONFIG_HINT) && err.contains("armonika.toml"),
        "{err}"
    );
    assert!(!out.contains(CONFIG_HINT), "{out}");

    // Only printing what would be removed gives no hint; removing does, also when the
    // name is typed with another case.
    let (ok, _, err) = armonika(dir.path(), &["library", "remove", "Jazz"], "");
    assert!(ok, "{err}");
    assert!(!err.contains(CONFIG_HINT), "{err}");
    let (ok, out, err) = armonika(dir.path(), &["library", "remove", "jazz", "--yes"], "");
    assert!(ok, "{out}{err}");
    assert!(
        err.contains(CONFIG_HINT) && err.contains("ARMONIKA_"),
        "{err}"
    );

    // Libraries that are not configured: no hint.
    let (ok, _, err) = armonika(dir.path(), &["library", "rename", "Other", "Spare"], "");
    assert!(ok, "{err}");
    assert!(!err.contains(CONFIG_HINT), "{err}");
    let (ok, _, err) = armonika(dir.path(), &["library", "remove", "Home", "--yes"], "");
    assert!(ok, "{err}");
    assert!(!err.contains(CONFIG_HINT), "{err}");
}

#[test]
fn user_grant_then_revoke() {
    let dir = tempfile::tempdir().unwrap();
    let music = mkdir(dir.path(), "music");
    let (ok, _, err) = armonika(dir.path(), &["library", "add", "Main", &music], "");
    assert!(ok, "{err}");
    let (ok, _, err) = armonika(
        dir.path(),
        &["user", "create", "bob", "--password-stdin"],
        "battery staple\n",
    );
    assert!(ok, "{err}");

    let (ok, out, err) = armonika(dir.path(), &["user", "grant", "bob", "Main", "listen"], "");
    assert!(ok, "{out}{err}");
    let (ok, out, err) = armonika(dir.path(), &["user", "grant", "bob", "Main", "manage"], "");
    assert!(ok, "{out}{err}");
    let (ok, _, _) = armonika(dir.path(), &["user", "grant", "bob", "Main", "own"], "");
    assert!(!ok, "unknown permission accepted");

    let (ok, out, err) = armonika(dir.path(), &["user", "revoke", "bob", "Main"], "");
    assert!(ok, "{out}{err}");
    assert!(out.contains("revoked"), "{out}");
    let (ok, out, err) = armonika(dir.path(), &["user", "revoke", "bob", "Main"], "");
    assert!(ok, "{out}{err}");
    assert!(out.contains("had no access"), "{out}");

    let (ok, _, err) = armonika(dir.path(), &["user", "grant", "bob", "Ghost", "listen"], "");
    assert!(!ok);
    assert!(err.contains("no library named \"Ghost\""), "{err}");
    let (ok, _, err) = armonika(dir.path(), &["user", "revoke", "bob", "Ghost"], "");
    assert!(!ok);
    assert!(err.contains("no library named \"Ghost\""), "{err}");
}

/// Starts `armonika serve` in `dir` with `env`, waits until it answers on `/`
/// (up to 10 s), stops it and returns whether it answered and what it logged.
fn serve_once(dir: &Path, env: &[(&str, &str)]) -> (bool, String) {
    use std::io::Read;
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let mut cmd = std::process::Command::new(assert_cmd::cargo::cargo_bin("armonika"));
    cmd.arg("serve")
        .env_remove("ARMONIKA_CONFIG")
        .env_remove("ARMONIKA_SECRET_KEY")
        .env_remove("ARMONIKA_ADMIN_USER")
        .env_remove("ARMONIKA_ADMIN_PASSWORD")
        .env("ARMONIKA_SERVER__BIND", format!("127.0.0.1:{port}"))
        .env("ARMONIKA_DATA_DIR", dir.join("data"))
        .current_dir(dir)
        .stderr(std::process::Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut answered = false;
    while std::time::Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            answered = true;
            break;
        }
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    let mut log = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut log)
        .unwrap();
    (answered, log)
}

#[test]
fn serve_bootstraps_admin_and_syncs_libraries() {
    let dir = tempfile::tempdir().unwrap();
    let music = mkdir(dir.path(), "music");
    std::fs::write(
        dir.path().join("armonika.toml"),
        format!("[[library]]\nname = \"Main\"\npath = '{music}'\n"),
    )
    .unwrap();

    let (answered, log) = serve_once(dir.path(), &[]);
    assert!(answered, "{log}");
    assert!(log.contains("ARMONIKA_ADMIN_USER"), "{log}");
    assert_single_spaced(&log, "no admin exists");
    // The library created from the configuration is logged with its path.
    let created = log
        .lines()
        .find(|l| l.contains("created library \"Main\""))
        .expect(&log);
    assert!(
        created.contains("INFO") && created.contains("music"),
        "{log}"
    );

    let password = "root-secret-password";
    let (answered, log) = serve_once(
        dir.path(),
        &[
            ("ARMONIKA_ADMIN_USER", "root"),
            ("ARMONIKA_ADMIN_PASSWORD", password),
        ],
    );
    assert!(answered, "{log}");
    assert!(log.contains("created the admin"), "{log}");
    assert!(!log.contains(password), "{log}");
    assert!(!log.contains("created library"), "{log}");

    let (_, out, _) = armonika(dir.path(), &["user", "list"], "");
    let root = out.lines().find(|l| l.contains("root")).expect(&out);
    assert!(root.contains("admin"), "{out}");
    let (_, out, _) = armonika(dir.path(), &["library", "list"], "");
    assert!(out.contains("Main"), "{out}");

    // The database wins over a changed path in the configuration, with a warning.
    let moved = mkdir(dir.path(), "moved");
    std::fs::write(
        dir.path().join("armonika.toml"),
        format!("[[library]]\nname = \"Main\"\npath = '{moved}'\n"),
    )
    .unwrap();
    // Admin variables set while an admin exists are ignored, said at info level.
    let (answered, log) = serve_once(
        dir.path(),
        &[
            ("ARMONIKA_ADMIN_USER", "other-admin-name"),
            ("ARMONIKA_ADMIN_PASSWORD", password),
        ],
    );
    assert!(answered, "{log}");
    assert!(log.contains("WARN") && log.contains("Main"), "{log}");
    let ignored = log.lines().find(|l| l.contains("ignored")).expect(&log);
    assert!(
        ignored.contains("INFO") && ignored.contains("armonika user set-password"),
        "{log}"
    );
    assert_single_spaced(&log, "ARMONIKA_ADMIN_USER or");
    assert!(
        !log.contains(password) && !log.contains("other-admin-name"),
        "{log}"
    );
    let (_, out, _) = armonika(dir.path(), &["library", "list"], "");
    let main = out.lines().find(|l| l.contains("Main")).expect(&out);
    assert!(main.contains("music") && !main.contains("moved"), "{out}");
}

#[test]
fn serve_keeps_running_when_admin_name_is_taken() {
    let dir = tempfile::tempdir().unwrap();
    let (ok, _, err) = armonika(
        dir.path(),
        &["user", "create", "root", "--password-stdin"],
        "plain-user-password\n",
    );
    assert!(ok, "{err}");
    let (answered, log) = serve_once(
        dir.path(),
        &[
            ("ARMONIKA_ADMIN_USER", "root"),
            ("ARMONIKA_ADMIN_PASSWORD", "another-password"),
        ],
    );
    assert!(answered, "{log}");
    assert!(
        log.contains("ERROR") && log.contains("not an admin"),
        "{log}"
    );
    assert!(!log.contains("another-password"), "{log}");
    assert_single_spaced(&log, "cannot create the admin");

    let (answered, log) = serve_once(
        dir.path(),
        &[
            ("ARMONIKA_ADMIN_USER", "boss"),
            ("ARMONIKA_ADMIN_PASSWORD", "sh0rt"),
        ],
    );
    assert!(answered, "{log}");
    assert!(log.contains("ERROR") && log.contains("at least"), "{log}");
    assert!(!log.contains("sh0rt"), "{log}");
    assert_single_spaced(&log, "cannot create the admin");
}

/// The log line holding `start` has no run of spaces from `start` to its end (the
/// formatter pads the level before it).
fn assert_single_spaced(log: &str, start: &str) {
    let line = log.lines().find(|l| l.contains(start)).expect(log);
    let message = &line[line.find(start).unwrap()..];
    assert!(
        !message.contains("  "),
        "two consecutive spaces in {message:?}"
    );
}
