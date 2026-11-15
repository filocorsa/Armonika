use armonika_core::Error;
use armonika_core::access::{self, Access, Permission};
use armonika_core::db::Db;
use armonika_core::ids::LibraryId;
use armonika_core::secret::SecretBox;
use armonika_core::users::{self, NewUser, UserFlagsUpdate};
use rusqlite::Connection;

fn open_temp() -> (tempfile::TempDir, Db, SecretBox) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(&dir.path().join("armonika.db")).unwrap();
    let sb = SecretBox::from_key([7; 32]);
    (dir, db, sb)
}

fn new_user(name: &str, is_admin: bool) -> NewUser {
    NewUser {
        name: name.to_owned(),
        password: format!("{name}-password"),
        is_admin,
        can_download: true,
    }
}

fn insert_library(conn: &Connection, name: &str) -> LibraryId {
    conn.execute(
        "INSERT INTO libraries (name, root, created_at) VALUES (?1, ?2, 0)",
        [name, &format!("/music/{name}")],
    )
    .unwrap();
    LibraryId(conn.last_insert_rowid())
}

#[test]
fn seal_open_roundtrip() {
    let sb = SecretBox::from_key([1; 32]);
    let sealed = sb.seal(b"hello subsonic");
    assert_eq!(sealed.len(), 24 + b"hello subsonic".len() + 16);
    assert!(!sealed.windows(5).any(|w| w == b"hello"));
    assert_eq!(sb.open(&sealed).unwrap(), b"hello subsonic");
    // A fresh random nonce each time.
    assert_ne!(sb.seal(b"hello subsonic"), sealed);
}

#[test]
fn open_rejects_tampered_ciphertext() {
    let sb = SecretBox::from_key([1; 32]);
    let mut sealed = sb.seal(b"hello subsonic");
    let last = sealed.len() - 1;
    sealed[last] ^= 1;
    assert!(matches!(sb.open(&sealed), Err(Error::Decrypt)));
    assert!(matches!(sb.open(&sealed[..10]), Err(Error::Decrypt)));
    let other = SecretBox::from_key([2; 32]);
    assert!(matches!(other.open(&sb.seal(b"x")), Err(Error::Decrypt)));
}

#[test]
fn secret_key_file_is_reused_across_loads() {
    let dir = tempfile::tempdir().unwrap();
    let first = SecretBox::load_or_create_with(dir.path(), None).unwrap();
    let key_file = dir.path().join("secret.key");
    assert!(key_file.is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&key_file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let sealed = first.seal(b"kept");
    let second = SecretBox::load_or_create_with(dir.path(), None).unwrap();
    assert_eq!(second.open(&sealed).unwrap(), b"kept");

    // The variable wins over the file.
    let from_env = SecretBox::load_or_create_with(
        dir.path(),
        Some("AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE="),
    )
    .unwrap();
    assert!(from_env.open(&sealed).is_err());
    assert_eq!(
        from_env
            .open(&SecretBox::from_key([1; 32]).seal(b"k"))
            .unwrap(),
        b"k"
    );
    assert!(matches!(
        SecretBox::load_or_create_with(dir.path(), Some("too-short")),
        Err(Error::InvalidSecretKey { .. })
    ));
}

#[test]
fn create_and_verify_password() {
    let (_dir, db, sb) = open_temp();
    db.write_blocking(|c| {
        let id = users::create(c, &sb, new_user("alice", false))?;
        assert_eq!(
            users::verify_password(c, "alice", "alice-password")?,
            Some(id.clone())
        );
        // Names are case-insensitive.
        assert_eq!(
            users::verify_password(c, "ALICE", "alice-password")?,
            Some(id.clone())
        );
        let hash: String = c.query_row("SELECT password_hash FROM users", [], |r| r.get(0))?;
        assert!(hash.starts_with("$argon2id$"), "{hash}");

        let user = users::by_name(c, "alice")?.unwrap();
        assert_eq!(user.id, id);
        assert!(!user.is_admin && user.can_download && !user.share_now_playing);

        assert!(matches!(
            users::create(c, &sb, new_user("Alice", false)),
            Err(Error::UserExists(_))
        ));
        let mut short = new_user("bob", false);
        short.password = "1234567".into();
        assert!(matches!(
            users::create(c, &sb, short),
            Err(Error::PasswordTooShort { .. })
        ));

        users::set_password(c, &id, "a brand new one")?;
        assert_eq!(users::verify_password(c, "alice", "alice-password")?, None);
        assert_eq!(
            users::verify_password(c, "alice", "a brand new one")?,
            Some(id)
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn wrong_password_is_rejected() {
    let (_dir, db, sb) = open_temp();
    db.write_blocking(|c| {
        users::create(c, &sb, new_user("alice", false))?;
        assert_eq!(users::verify_password(c, "alice", "nope-nope-nope")?, None);
        assert_eq!(users::verify_password(c, "alice", "")?, None);
        assert_eq!(users::verify_password(c, "nobody", "alice-password")?, None);
        Ok(())
    })
    .unwrap();
}

#[test]
fn subsonic_password_is_recoverable() {
    let (_dir, db, sb) = open_temp();
    db.write_blocking(|c| {
        let id = users::create(c, &sb, new_user("alice", false))?;
        // The initial Subsonic password is the login password.
        assert_eq!(users::subsonic_password(c, &sb, &id)?, "alice-password");
        let stored: Vec<u8> =
            c.query_row("SELECT subsonic_password FROM users", [], |r| r.get(0))?;
        assert!(!stored.windows(5).any(|w| w == b"alice"));

        users::set_subsonic_password(c, &sb, &id, "subsonic-only")?;
        assert_eq!(users::subsonic_password(c, &sb, &id)?, "subsonic-only");
        // The login password is unchanged.
        assert_eq!(
            users::verify_password(c, "alice", "alice-password")?,
            Some(id)
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn cannot_delete_last_admin() {
    let (_dir, db, sb) = open_temp();
    db.write_blocking(|c| {
        let admin = users::create(c, &sb, new_user("admin", true))?;
        let alice = users::create(c, &sb, new_user("alice", false))?;
        assert!(matches!(users::delete(c, &admin), Err(Error::LastAdmin)));
        users::delete(c, &alice)?;
        assert!(users::by_name(c, "alice")?.is_none());
        assert!(matches!(
            users::delete(c, &alice),
            Err(Error::UserNotFound(_))
        ));

        let second = users::create(c, &sb, new_user("second", true))?;
        users::delete(c, &admin)?;
        assert!(matches!(users::delete(c, &second), Err(Error::LastAdmin)));
        Ok(())
    })
    .unwrap();
}

#[test]
fn cannot_demote_last_admin() {
    let (_dir, db, sb) = open_temp();
    db.write_blocking(|c| {
        let admin = users::create(c, &sb, new_user("admin", true))?;
        let demote = UserFlagsUpdate {
            is_admin: Some(false),
            can_download: Some(false),
            share_now_playing: None,
        };
        assert!(matches!(
            users::set_flags(c, &admin, demote.clone()),
            Err(Error::LastAdmin)
        ));
        // Nothing changed, not even the other flags.
        let user = users::by_name(c, "admin")?.unwrap();
        assert!(user.is_admin && user.can_download);

        let alice = users::create(c, &sb, new_user("alice", false))?;
        users::set_flags(
            c,
            &alice,
            UserFlagsUpdate {
                is_admin: Some(true),
                can_download: None,
                share_now_playing: Some(true),
            },
        )?;
        users::set_flags(c, &admin, demote)?;
        let admin_now = users::by_name(c, "admin")?.unwrap();
        assert!(!admin_now.is_admin && !admin_now.can_download);
        let alice_now = users::by_name(c, "alice")?.unwrap();
        assert!(alice_now.is_admin && alice_now.share_now_playing);
        assert_eq!(users::list(c)?.len(), 2);
        Ok(())
    })
    .unwrap();
}

#[test]
fn api_key_lookup_uses_hash_only() {
    let (_dir, db, sb) = open_temp();
    db.write_blocking(|c| {
        let id = users::create(c, &sb, new_user("alice", false))?;
        let key = users::create_api_key(c, &id, "phone")?;
        assert_eq!(key.len(), 43, "32 bytes in base64url without padding");
        assert!(matches!(
            users::create_api_key(c, &id, "phone"),
            Err(Error::ApiKeyLabelExists(_))
        ));

        let rows: Vec<(String, String)> = c
            .prepare("SELECT key_hash, label FROM api_keys")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].0.contains(&key));
        assert_eq!(rows[0].0, blake3::hash(key.as_bytes()).to_hex().as_str());

        assert_eq!(users::user_by_api_key(c, &key)?, Some(id.clone()));
        assert_eq!(users::user_by_api_key(c, "not-a-key")?, None);

        users::revoke_api_key(c, &id, "phone")?;
        assert_eq!(users::user_by_api_key(c, &key)?, None);
        assert!(matches!(
            users::revoke_api_key(c, &id, "phone"),
            Err(Error::ApiKeyNotFound(_))
        ));
        Ok(())
    })
    .unwrap();
}

#[test]
fn bootstrap_admin_creates_only_when_no_admin() {
    let (_dir, db, sb) = open_temp();
    db.write_blocking(|c| {
        assert_eq!(
            users::bootstrap_admin(c, &sb, None, None)?,
            users::BootstrapOutcome::MissingCredentials
        );
        assert_eq!(
            users::bootstrap_admin(c, &sb, Some("root".into()), None)?,
            users::BootstrapOutcome::MissingCredentials
        );
        let outcome =
            users::bootstrap_admin(c, &sb, Some("root".into()), Some("root-password".into()))?;
        let users::BootstrapOutcome::Created(id) = outcome else {
            panic!("admin not created: {outcome:?}");
        };
        assert!(users::by_name(c, "root")?.unwrap().is_admin);
        assert_eq!(
            users::verify_password(c, "root", "root-password")?,
            Some(id)
        );
        assert_eq!(
            users::bootstrap_admin(c, &sb, Some("other".into()), Some("other-password".into()))?,
            users::BootstrapOutcome::AdminExists
        );
        assert!(users::by_name(c, "other")?.is_none());
        Ok(())
    })
    .unwrap();
}

#[test]
fn admin_sees_all_libraries_with_manage() {
    let (_dir, db, sb) = open_temp();
    db.write_blocking(|c| {
        let admin = users::create(c, &sb, new_user("admin", true))?;
        let a = insert_library(c, "a");
        let b = insert_library(c, "b");
        let access = Access::load(c, &admin)?;
        assert_eq!(access.user, admin);
        assert_eq!(access.name, "admin");
        assert!(access.is_admin && access.can_download);
        assert_eq!(
            access.libraries,
            vec![(a, Permission::Manage), (b, Permission::Manage)]
        );
        assert_eq!(access.visible(None), vec![a, b]);
        assert_eq!(access.visible(Some(b)), vec![b]);
        Ok(())
    })
    .unwrap();
}

#[test]
fn new_user_sees_no_library() {
    let (_dir, db, sb) = open_temp();
    db.write_blocking(|c| {
        let alice = users::create(c, &sb, new_user("alice", false))?;
        let a = insert_library(c, "a");
        let b = insert_library(c, "b");
        let access = Access::load(c, &alice)?;
        assert!(access.libraries.is_empty());
        assert!(access.visible(None).is_empty());

        access::grant(c, &alice, a, Permission::Listen)?;
        access::grant(c, &alice, b, Permission::Listen)?;
        access::grant(c, &alice, b, Permission::Manage)?;
        let access = Access::load(c, &alice)?;
        assert_eq!(
            access.libraries,
            vec![(a, Permission::Listen), (b, Permission::Manage)]
        );

        assert!(access::revoke(c, &alice, a)?);
        assert!(!access::revoke(c, &alice, a)?);
        assert_eq!(Access::load(c, &alice)?.visible(None), vec![b]);

        assert!(matches!(
            access::grant(c, &alice, LibraryId(999), Permission::Listen),
            Err(Error::LibraryNotFound(_))
        ));
        let ghost = armonika_core::ids::UserId::generate();
        assert!(matches!(
            Access::load(c, &ghost),
            Err(Error::UserNotFound(_))
        ));
        Ok(())
    })
    .unwrap();
}

#[test]
fn visible_with_foreign_filter_is_empty() {
    let (_dir, db, sb) = open_temp();
    db.write_blocking(|c| {
        let alice = users::create(c, &sb, new_user("alice", false))?;
        let mine = insert_library(c, "mine");
        let foreign = insert_library(c, "foreign");
        access::grant(c, &alice, mine, Permission::Listen)?;
        let access = Access::load(c, &alice)?;
        assert_eq!(access.visible(Some(mine)), vec![mine]);
        assert!(access.visible(Some(foreign)).is_empty());
        assert!(access.visible(Some(LibraryId(12345))).is_empty());
        Ok(())
    })
    .unwrap();
}

#[test]
fn unparsable_stored_hash_is_an_error_not_a_wrong_password() {
    let (_dir, db, sb) = open_temp();
    db.write_blocking(|c| {
        users::create(c, &sb, new_user("ann", false))?;
        c.execute(
            "UPDATE users SET password_hash = 'not-a-phc-string' WHERE name = 'ann'",
            [],
        )?;
        let err = users::verify_password(c, "ann", "ann-password").unwrap_err();
        assert!(matches!(err, Error::PasswordHash(_)), "{err:?}");
        // A user that does not exist is still just "no match".
        assert_eq!(users::verify_password(c, "ghost", "whatever-pw")?, None);
        Ok(())
    })
    .unwrap();
}

#[test]
fn unparsable_permission_is_an_error_not_listen() {
    let (_dir, db, sb) = open_temp();
    db.write_blocking(|c| {
        let ann = users::create(c, &sb, new_user("ann", false))?;
        let lib = insert_library(c, "A");
        access::grant(c, &ann, lib, Permission::Manage)?;
        c.execute_batch("PRAGMA ignore_check_constraints = ON")?;
        c.execute("UPDATE library_access SET permission = 'bogus'", [])?;
        let err = Access::load(c, &ann).unwrap_err();
        assert!(matches!(err, Error::InvalidPermission(_)), "{err:?}");
        Ok(())
    })
    .unwrap();
}
