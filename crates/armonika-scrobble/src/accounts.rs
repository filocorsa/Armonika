//! The scrobbling accounts of the users: one sealed token per user and service.

use armonika_core::db::Connection;
use armonika_core::ids::{UserId, resolve_user};
use armonika_core::secret::SecretBox;
use armonika_core::{Error, Result};
use rusqlite::{OptionalExtension, params};

fn require_user(conn: &Connection, user: &UserId) -> Result<i64> {
    resolve_user(conn, user)?.ok_or_else(|| Error::UserNotFound(user.to_string()))
}

/// Stores `token` of `service` for `user`, sealed with the server key, replacing an
/// earlier one. Rows waiting in the outbox for the service become due at once, with their attempts reset, so a
/// new token after a refused one sends what was kept.
pub fn link(
    conn: &Connection,
    sb: &SecretBox,
    user: &UserId,
    service: &str,
    token: &str,
    account_name: Option<&str>,
) -> Result<()> {
    let user = require_user(conn, user)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO scrobble_accounts (user_id, service, secret, account_name)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (user_id, service) DO UPDATE SET
           secret = excluded.secret, account_name = excluded.account_name",
        params![user, service, sb.seal(token.as_bytes()), account_name],
    )?;
    tx.execute(
        "UPDATE scrobble_outbox SET attempts = 0, next_attempt_at = 0 WHERE user_id = ?1 AND service = ?2",
        params![user, service],
    )?;
    tx.commit()?;
    Ok(())
}

/// Removes the token of `service` of `user` and the plays still waiting for that
/// service. Returns whether there was an account.
pub fn unlink(conn: &Connection, user: &UserId, service: &str) -> Result<bool> {
    let user = require_user(conn, user)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM scrobble_outbox WHERE user_id = ?1 AND service = ?2",
        params![user, service],
    )?;
    let removed = tx.execute(
        "DELETE FROM scrobble_accounts WHERE user_id = ?1 AND service = ?2",
        params![user, service],
    )?;
    tx.commit()?;
    Ok(removed > 0)
}

/// The token of `service` of `user`, if linked.
pub fn token(
    conn: &Connection,
    sb: &SecretBox,
    user: &UserId,
    service: &str,
) -> Result<Option<String>> {
    let user = require_user(conn, user)?;
    token_by_key(conn, sb, user, service)
}

/// [`token`] by the internal key of the user.
pub(crate) fn token_by_key(
    conn: &Connection,
    sb: &SecretBox,
    user: i64,
    service: &str,
) -> Result<Option<String>> {
    let sealed: Option<Vec<u8>> = conn
        .prepare_cached("SELECT secret FROM scrobble_accounts WHERE user_id = ?1 AND service = ?2")?
        .query_row(params![user, service], |r| r.get(0))
        .optional()?;
    sealed.map(|s| open(sb, &s)).transpose()
}

/// Every `(service, token)` of the user with internal key `user`.
pub(crate) fn tokens_of(
    conn: &Connection,
    sb: &SecretBox,
    user: i64,
) -> Result<Vec<(String, String)>> {
    let rows: Vec<(String, Vec<u8>)> = conn
        .prepare_cached("SELECT service, secret FROM scrobble_accounts WHERE user_id = ?1")?
        .query_map([user], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(|(service, sealed)| Ok((service, open(sb, &sealed)?)))
        .collect()
}

fn open(sb: &SecretBox, sealed: &[u8]) -> Result<String> {
    String::from_utf8(sb.open(sealed)?).map_err(|_| Error::Decrypt)
}
