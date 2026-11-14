use armonika_core::Error;
use armonika_core::secret::{self, KeySource, SecretBox};

#[test]
fn invalid_key_names_its_source_never_the_value() {
    let dir = tempfile::tempdir().unwrap();
    let err = SecretBox::load_or_create_with(dir.path(), Some("too-short-value")).unwrap_err();
    assert!(
        matches!(
            &err,
            Error::InvalidSecretKey {
                from: KeySource::Env
            }
        ),
        "{err:?}"
    );
    let text = err.to_string();
    assert!(text.contains("ARMONIKA_SECRET_KEY"), "{text}");
    assert!(!text.contains("too-short-value"), "{text}");

    let key_file = dir.path().join("secret.key");
    std::fs::write(&key_file, "bad-file-value\n").unwrap();
    let err = SecretBox::load_or_create_with(dir.path(), None).unwrap_err();
    assert!(
        matches!(&err, Error::InvalidSecretKey { from: KeySource::File(p) } if p == &key_file),
        "{err:?}"
    );
    let text = err.to_string();
    assert!(text.contains(&key_file.display().to_string()), "{text}");
    assert!(!text.contains("bad-file-value"), "{text}");
}

#[test]
fn empty_key_file_is_reported_not_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let key_file = dir.path().join("secret.key");
    std::fs::write(&key_file, "").unwrap();
    let err = SecretBox::load_or_create_with(dir.path(), None).unwrap_err();
    assert!(
        matches!(
            &err,
            Error::InvalidSecretKey {
                from: KeySource::File(_)
            }
        ),
        "{err:?}"
    );
    assert_eq!(std::fs::read(&key_file).unwrap(), b"");
}

#[test]
fn key_file_creation_leaves_no_temporary_file() {
    let dir = tempfile::tempdir().unwrap();
    let first = SecretBox::load_or_create_with(dir.path(), None).unwrap();
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["secret.key"]);
    let sealed = first.seal(b"kept");
    let again = SecretBox::load_or_create_with(dir.path(), None).unwrap();
    assert_eq!(again.open(&sealed).unwrap(), b"kept");
}

#[test]
fn concurrent_first_starts_agree_on_one_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let boxes: Vec<SecretBox> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..8)
            .map(|_| s.spawn(|| SecretBox::load_or_create_with(&path, None).unwrap()))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let sealed = boxes[0].seal(b"same key");
    for b in &boxes {
        assert_eq!(b.open(&sealed).unwrap(), b"same key");
    }
    let count = std::fs::read_dir(dir.path()).unwrap().count();
    assert_eq!(count, 1, "temporary files left behind");
}

fn key_b64(byte: u8) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode([byte; 32])
}

#[test]
fn canary_is_created_then_accepts_the_same_key() {
    let dir = tempfile::tempdir().unwrap();
    let db = armonika_core::db::Db::open(&dir.path().join("a.db")).unwrap();
    let sb = SecretBox::load_or_create_with(dir.path(), Some(&key_b64(1))).unwrap();
    db.write_blocking(|c| secret::check_canary(c, &sb)).unwrap();
    db.write_blocking(|c| secret::check_canary(c, &sb)).unwrap();
    let again = SecretBox::load_or_create_with(dir.path(), Some(&key_b64(1))).unwrap();
    db.write_blocking(|c| secret::check_canary(c, &again))
        .unwrap();
}

#[test]
fn canary_detects_a_changed_key_and_names_the_source() {
    let dir = tempfile::tempdir().unwrap();
    let db = armonika_core::db::Db::open(&dir.path().join("a.db")).unwrap();
    let first = SecretBox::load_or_create_with(dir.path(), Some(&key_b64(1))).unwrap();
    db.write_blocking(|c| secret::check_canary(c, &first))
        .unwrap();
    let other = SecretBox::load_or_create_with(dir.path(), Some(&key_b64(2))).unwrap();
    let err = db
        .write_blocking(|c| secret::check_canary(c, &other))
        .unwrap_err();
    assert!(matches!(err, Error::SecretKeyMismatch { .. }), "{err:?}");
    let text = err.to_string();
    assert!(text.contains("ARMONIKA_SECRET_KEY"), "{text}");
    assert!(!text.contains(&key_b64(2)), "{text}");
    // A key file as source is named by its path.
    let file_box = SecretBox::load_or_create_with(dir.path(), None).unwrap();
    let err = db
        .write_blocking(|c| secret::check_canary(c, &file_box))
        .unwrap_err();
    assert!(err.to_string().contains("secret.key"), "{err}");
}

#[test]
fn canary_without_row_checks_existing_subsonic_passwords() {
    use armonika_core::users::{self, NewUser};
    let dir = tempfile::tempdir().unwrap();
    let db = armonika_core::db::Db::open(&dir.path().join("a.db")).unwrap();
    let first = SecretBox::from_key([1; 32]);
    db.write_blocking(|c| {
        users::create(
            c,
            &first,
            NewUser {
                name: "ann".to_owned(),
                password: "ann-password".to_owned(),
                is_admin: false,
                can_download: true,
            },
        )
    })
    .unwrap();
    // A database from before the canary: the key is checked against a user.
    let other = SecretBox::from_key([2; 32]);
    let err = db
        .write_blocking(|c| secret::check_canary(c, &other))
        .unwrap_err();
    assert!(matches!(err, Error::SecretKeyMismatch { .. }), "{err:?}");
    db.write_blocking(|c| secret::check_canary(c, &first))
        .unwrap();
    // Now the canary exists and rejects the other key too.
    assert!(
        db.write_blocking(|c| secret::check_canary(c, &other))
            .is_err()
    );
}
