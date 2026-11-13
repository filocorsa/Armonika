use armonika_core::Error;
use armonika_core::secret::{KeySource, SecretBox};

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
