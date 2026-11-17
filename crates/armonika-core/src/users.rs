//! Users: accounts, passwords, flags and API keys.
//!
//! The login password is stored as an argon2id hash. The Subsonic password is stored
//! sealed with the [`SecretBox`], because the Subsonic token authentication needs it in
//! clear; at creation it is the login password. API keys are stored as blake3 hashes.

use std::sync::OnceLock;

use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rusqlite::{Connection, ErrorCode, OptionalExtension, params};

use crate::db::now_ms;
use crate::ids::{UserId, resolve_user};
use crate::secret::{SecretBox, random_bytes};
use crate::{Error, Result};

/// Minimum length of a password, in characters.
pub const MIN_PASSWORD_CHARS: usize = 8;

/// A user to create.
#[derive(Clone)]
pub struct NewUser {
    /// Login name, unique without regard to case.
    pub name: String,
    /// Login password; also the initial Subsonic password.
    pub password: String,
    /// Whether the user administers the server.
    pub is_admin: bool,
    /// Whether the user may download original files.
    pub can_download: bool,
}

impl std::fmt::Debug for NewUser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewUser")
            .field("name", &self.name)
            .field("is_admin", &self.is_admin)
            .field("can_download", &self.can_download)
            .finish_non_exhaustive()
    }
}

/// A user as listed by [`by_name`] and [`list`]. Holds no secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    /// Public identifier.
    pub id: UserId,
    /// Login name.
    pub name: String,
    /// Whether the user administers the server.
    pub is_admin: bool,
    /// Whether the user may download original files.
    pub can_download: bool,
    /// Whether other users can see what this user is playing.
    pub share_now_playing: bool,
    /// Creation time, Unix milliseconds.
    pub created_at: i64,
}

/// Flags to change with [`set_flags`]; `None` leaves a flag as it is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserFlagsUpdate {
    /// New value of `is_admin`.
    pub is_admin: Option<bool>,
    /// New value of `can_download`.
    pub can_download: Option<bool>,
    /// New value of `share_now_playing`.
    pub share_now_playing: Option<bool>,
}

/// What [`bootstrap_admin`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BootstrapOutcome {
    /// An admin already exists; nothing was done.
    AdminExists,
    /// No admin existed and one was created from the given credentials.
    Created(UserId),
    /// No admin exists and the name or the password was not given.
    MissingCredentials,
}

/// Creates a user and returns its identifier.
pub fn create(conn: &Connection, sb: &SecretBox, user: NewUser) -> Result<UserId> {
    check_name(&user.name)?;
    check_password(&user.password)?;
    if by_name(conn, &user.name)?.is_some() {
        return Err(Error::UserExists(user.name));
    }
    let hash = hash_password(&user.password)?;
    let sealed = sb.seal(user.password.as_bytes());
    let id = UserId::generate();
    let inserted = conn.execute(
        "INSERT INTO users (public_id, name, password_hash, subsonic_password, is_admin,
                            can_download, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            id,
            user.name,
            hash,
            sealed,
            user.is_admin,
            user.can_download,
            now_ms()
        ],
    );
    match inserted {
        Ok(_) => Ok(id),
        Err(e) if e.sqlite_error_code() == Some(ErrorCode::ConstraintViolation) => {
            Err(Error::UserExists(user.name))
        }
        Err(e) => Err(e.into()),
    }
}

/// Checks a login. Returns the user if `name` exists and `password` matches.
pub fn verify_password(conn: &Connection, name: &str, password: &str) -> Result<Option<UserId>> {
    let row: Option<(UserId, String)> = conn
        .prepare_cached("SELECT public_id, password_hash FROM users WHERE name = ?1")?
        .query_row([name], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    let Some((id, hash)) = row else {
        // Spend the same time as for a real user, so timing does not tell which
        // names exist.
        check_hash(password, dummy_hash()?)?;
        return Ok(None);
    };
    // A stored hash that cannot be parsed is a damaged row, not a wrong password.
    Ok(check_hash(password, &hash)?.then_some(id))
}

/// Whether `password` matches the PHC string `hash`; an error if `hash` is unusable.
fn check_hash(password: &str, hash: &str) -> Result<bool> {
    match Argon2::default().verify_password(password.as_bytes(), hash) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::PasswordInvalid) => Ok(false),
        Err(e) => Err(Error::PasswordHash(e.to_string())),
    }
}

/// The Subsonic password of `user`, in clear.
pub fn subsonic_password(conn: &Connection, sb: &SecretBox, user: &UserId) -> Result<String> {
    let key = require_user(conn, user)?;
    let sealed: Vec<u8> = conn.query_row(
        "SELECT subsonic_password FROM users WHERE id = ?1",
        [key],
        |r| r.get(0),
    )?;
    String::from_utf8(sb.open(&sealed)?).map_err(|_| Error::Decrypt)
}

/// Changes the login password of `user`. The Subsonic password is not changed.
pub fn set_password(conn: &Connection, user: &UserId, password: &str) -> Result<()> {
    check_password(password)?;
    let key = require_user(conn, user)?;
    let hash = hash_password(password)?;
    conn.execute(
        "UPDATE users SET password_hash = ?2 WHERE id = ?1",
        params![key, hash],
    )?;
    Ok(())
}

/// Changes the Subsonic password of `user`. The login password is not changed.
pub fn set_subsonic_password(
    conn: &Connection,
    sb: &SecretBox,
    user: &UserId,
    password: &str,
) -> Result<()> {
    check_password(password)?;
    let key = require_user(conn, user)?;
    conn.execute(
        "UPDATE users SET subsonic_password = ?2 WHERE id = ?1",
        params![key, sb.seal(password.as_bytes())],
    )?;
    Ok(())
}

/// Changes the flags of `user`. Fails with [`Error::LastAdmin`], changing nothing, if
/// it would remove `is_admin` from the last admin.
pub fn set_flags(conn: &Connection, user: &UserId, flags: UserFlagsUpdate) -> Result<()> {
    let key = require_user(conn, user)?;
    // One statement, so the last-admin check and the change are atomic.
    let changed = conn.execute(
        "UPDATE users SET
           is_admin = COALESCE(?2, is_admin),
           can_download = COALESCE(?3, can_download),
           share_now_playing = COALESCE(?4, share_now_playing)
         WHERE id = ?1
           AND (?2 IS NOT 0 OR is_admin = 0
                OR EXISTS (SELECT 1 FROM users WHERE is_admin = 1 AND id != ?1))",
        params![
            key,
            flags.is_admin,
            flags.can_download,
            flags.share_now_playing
        ],
    )?;
    if changed == 0 {
        return Err(Error::LastAdmin);
    }
    Ok(())
}

/// Deletes `user` and everything that belongs to it. Fails with [`Error::LastAdmin`] if
/// `user` is the last admin.
pub fn delete(conn: &Connection, user: &UserId) -> Result<()> {
    let key = require_user(conn, user)?;
    let changed = conn.execute(
        "DELETE FROM users WHERE id = ?1
           AND (is_admin = 0 OR EXISTS (SELECT 1 FROM users WHERE is_admin = 1 AND id != ?1))",
        [key],
    )?;
    if changed == 0 {
        return Err(Error::LastAdmin);
    }
    Ok(())
}

/// The user called `name`, compared without case.
pub fn by_name(conn: &Connection, name: &str) -> Result<Option<User>> {
    Ok(conn
        .prepare_cached(&format!("{USER_COLUMNS} WHERE name = ?1"))?
        .query_row([name], user_from_row)
        .optional()?)
}

/// Every user, by name.
pub fn list(conn: &Connection) -> Result<Vec<User>> {
    Ok(conn
        .prepare_cached(&format!("{USER_COLUMNS} ORDER BY name"))?
        .query_map([], user_from_row)?
        .collect::<rusqlite::Result<_>>()?)
}

/// Creates an API key for `user` and returns it. The key is 32 random bytes in
/// base64url; only its hash is stored, so this is the only time it can be read.
pub fn create_api_key(conn: &Connection, user: &UserId, label: &str) -> Result<String> {
    let key = require_user(conn, user)?;
    let mut bytes = [0; 32];
    random_bytes(&mut bytes)?;
    let api_key = URL_SAFE_NO_PAD.encode(bytes);
    let inserted = conn.execute(
        "INSERT INTO api_keys (key_hash, user_id, label, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![api_key_hash(&api_key), key, label, now_ms()],
    );
    match inserted {
        Ok(_) => Ok(api_key),
        // UNIQUE (user_id, label); a clash on the key hash itself would be a
        // SQLITE_CONSTRAINT_PRIMARYKEY instead.
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE =>
        {
            Err(Error::ApiKeyLabelExists(label.to_owned()))
        }
        Err(e) => Err(e.into()),
    }
}

/// The labels of the API keys of `user`, by label. The keys themselves are not stored.
pub fn api_key_labels(conn: &Connection, user: &UserId) -> Result<Vec<String>> {
    let key = require_user(conn, user)?;
    Ok(conn
        .prepare_cached("SELECT label FROM api_keys WHERE user_id = ?1 ORDER BY label")?
        .query_map([key], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

/// The owner of the API key `api_key`, if it exists.
pub fn user_by_api_key(conn: &Connection, api_key: &str) -> Result<Option<UserId>> {
    Ok(conn
        .prepare_cached(
            "SELECT u.public_id FROM api_keys k JOIN users u ON u.id = k.user_id
             WHERE k.key_hash = ?1",
        )?
        .query_row([api_key_hash(api_key)], |r| r.get(0))
        .optional()?)
}

/// Deletes the API key labelled `label` of `user`.
pub fn revoke_api_key(conn: &Connection, user: &UserId, label: &str) -> Result<()> {
    let key = require_user(conn, user)?;
    let deleted = conn.execute(
        "DELETE FROM api_keys WHERE user_id = ?1 AND label = ?2",
        params![key, label],
    )?;
    if deleted == 0 {
        return Err(Error::ApiKeyNotFound(label.to_owned()));
    }
    Ok(())
}

/// Creates the first admin at startup. If an admin exists nothing is done; otherwise
/// the admin is created from `name` and `password` (the values of
/// `ARMONIKA_ADMIN_USER` and `ARMONIKA_ADMIN_PASSWORD`, read by the caller) when both
/// are given.
pub fn bootstrap_admin(
    conn: &Connection,
    sb: &SecretBox,
    name: Option<String>,
    password: Option<String>,
) -> Result<BootstrapOutcome> {
    let has_admin: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM users WHERE is_admin = 1)",
        [],
        |r| r.get(0),
    )?;
    if has_admin {
        return Ok(BootstrapOutcome::AdminExists);
    }
    let (Some(name), Some(password)) = (name, password) else {
        return Ok(BootstrapOutcome::MissingCredentials);
    };
    let id = create(
        conn,
        sb,
        NewUser {
            name,
            password,
            is_admin: true,
            can_download: true,
        },
    )?;
    Ok(BootstrapOutcome::Created(id))
}

/// Internal key of `user`, or [`Error::UserNotFound`].
pub(crate) fn require_user(conn: &Connection, user: &UserId) -> Result<i64> {
    resolve_user(conn, user)?.ok_or_else(|| Error::UserNotFound(user.to_string()))
}

const USER_COLUMNS: &str =
    "SELECT public_id, name, is_admin, can_download, share_now_playing, created_at FROM users";

fn user_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    Ok(User {
        id: r.get(0)?,
        name: r.get(1)?,
        is_admin: r.get(2)?,
        can_download: r.get(3)?,
        share_now_playing: r.get(4)?,
        created_at: r.get(5)?,
    })
}

fn check_name(name: &str) -> Result<()> {
    if name.is_empty() || name.trim() != name || name.chars().any(char::is_control) {
        return Err(Error::InvalidUserName(name.to_owned()));
    }
    Ok(())
}

fn check_password(password: &str) -> Result<()> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(Error::PasswordTooShort {
            min: MIN_PASSWORD_CHARS,
        });
    }
    Ok(())
}

/// argon2id with the default parameters of the `argon2` crate, as a PHC string.
fn hash_password(password: &str) -> Result<String> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| Error::PasswordHash(e.to_string()))
}

/// A valid hash of a random password, verified against when the user does not exist.
fn dummy_hash() -> Result<&'static str> {
    static HASH: OnceLock<std::result::Result<String, String>> = OnceLock::new();
    HASH.get_or_init(|| {
        let mut bytes = [0; 16];
        random_bytes(&mut bytes).map_err(|e| e.to_string())?;
        hash_password(&URL_SAFE_NO_PAD.encode(bytes)).map_err(|e| e.to_string())
    })
    .as_deref()
    .map_err(|e| Error::PasswordHash(e.clone()))
}

fn api_key_hash(api_key: &str) -> String {
    blake3::hash(api_key.as_bytes()).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dummy_hash_is_a_real_hash_that_costs_a_full_verification() {
        let hash = dummy_hash().unwrap();
        assert!(argon2::password_hash::phc::PasswordHash::new(hash).is_ok());
        assert!(!check_hash("not the random password", hash).unwrap());
    }
}
