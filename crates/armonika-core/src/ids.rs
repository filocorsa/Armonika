//! Public identifiers and their translation into internal keys.
//!
//! Every entity has two identities in the database: the internal key `id INTEGER`, used
//! by the links between tables, and the stable `public_id` text column, the only one
//! exposed by the APIs. Boundary rule: public functions of the core take and return the
//! public identifiers defined here; the `resolve_*` functions are the only place where a
//! public identifier becomes an internal key, and internal keys never leave the core.

use std::fmt;
use std::str::FromStr;

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::db::now_ms;
use crate::{Error, Result};

macro_rules! public_id {
    ($(#[$doc:meta])* $name:ident, $prefix:literal) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            /// Prefix of this kind of identifier, without the dash.
            pub const PREFIX: &'static str = $prefix;

            /// A new identifier: the prefix, a dash and a UUID v7 in hex without dashes.
            pub fn generate() -> Self {
                Self(format!("{}-{}", $prefix, uuid::Uuid::now_v7().simple()))
            }

            /// The identifier as stored in `public_id` and exposed in the APIs.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl FromStr for $name {
            type Err = Error;

            fn from_str(s: &str) -> Result<Self> {
                if has_prefix(s, $prefix) {
                    Ok(Self(s.to_owned()))
                } else {
                    Err(Error::InvalidId(s.to_owned()))
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl ToSql for $name {
            fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
                self.0.to_sql()
            }
        }

        impl FromSql for $name {
            fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
                let s = value.as_str()?;
                s.parse().map_err(|e| FromSqlError::Other(Box::new(e)))
            }
        }
    };
}

public_id!(
    /// Public identifier of a track (`t-...`).
    TrackId,
    "t"
);
public_id!(
    /// Public identifier of a library album (`al-...`).
    LibraryAlbumId,
    "al"
);
public_id!(
    /// Public identifier of an artist (`ar-...`).
    ArtistId,
    "ar"
);
public_id!(
    /// Public identifier of a user (`u-...`).
    UserId,
    "u"
);
public_id!(
    /// Public identifier of a playlist (`pl-...`).
    PlaylistId,
    "pl"
);

/// True if `s` is `prefix`, a dash and a non-empty ASCII alphanumeric rest.
fn has_prefix(s: &str, prefix: &str) -> bool {
    s.strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix('-'))
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// Identifier of a library. Libraries keep their integer key as public identifier
/// because Subsonic apps expect an integer for music folders.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LibraryId(pub i64);

impl FromStr for LibraryId {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        s.parse()
            .map(LibraryId)
            .map_err(|_| Error::InvalidId(s.to_owned()))
    }
}

impl fmt::Display for LibraryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A browsable item that users can annotate and search: a track, an album or an artist.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ItemRef {
    /// A track.
    Track(TrackId),
    /// A library album.
    Album(LibraryAlbumId),
    /// An artist.
    Artist(ArtistId),
}

impl ItemRef {
    /// The public identifier of the item.
    pub fn as_str(&self) -> &str {
        match self {
            ItemRef::Track(id) => id.as_str(),
            ItemRef::Album(id) => id.as_str(),
            ItemRef::Artist(id) => id.as_str(),
        }
    }
}

impl FromStr for ItemRef {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        if let Ok(id) = s.parse() {
            Ok(ItemRef::Track(id))
        } else if let Ok(id) = s.parse() {
            Ok(ItemRef::Album(id))
        } else if let Ok(id) = s.parse() {
            Ok(ItemRef::Artist(id))
        } else {
            Err(Error::InvalidId(s.to_owned()))
        }
    }
}

impl fmt::Display for ItemRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Internal key of the track `id`, following a redirect if the track was merged.
/// The key is for queries inside the core only.
pub fn resolve_track(conn: &Connection, id: &TrackId) -> Result<Option<i64>> {
    resolve(conn, "tracks", id.as_str())
}

/// Internal key of the library album `id`, following a redirect if the album was merged.
/// The key is for queries inside the core only.
pub fn resolve_album(conn: &Connection, id: &LibraryAlbumId) -> Result<Option<i64>> {
    resolve(conn, "library_albums", id.as_str())
}

/// Internal key of the artist `id`, following a redirect if the artist was merged.
/// The key is for queries inside the core only.
pub fn resolve_artist(conn: &Connection, id: &ArtistId) -> Result<Option<i64>> {
    resolve(conn, "artists", id.as_str())
}

/// Internal key of the user `id`. The key is for queries inside the core only.
pub fn resolve_user(conn: &Connection, id: &UserId) -> Result<Option<i64>> {
    resolve(conn, "users", id.as_str())
}

/// Internal key of the playlist `id`. The key is for queries inside the core only.
pub fn resolve_playlist(conn: &Connection, id: &PlaylistId) -> Result<Option<i64>> {
    resolve(conn, "playlists", id.as_str())
}

/// Looks `public_id` up in `table`, then through `id_redirects`. Redirect chains are
/// kept one step long by [`add_redirect`], so one lookup of the redirect is enough.
fn resolve(conn: &Connection, table: &'static str, public_id: &str) -> Result<Option<i64>> {
    let sql = format!("SELECT id FROM {table} WHERE public_id = ?1");
    let mut stmt = conn.prepare_cached(&sql)?;
    if let Some(key) = stmt.query_row([public_id], |r| r.get(0)).optional()? {
        return Ok(Some(key));
    }
    let Some(target) = redirect_target(conn, public_id)? else {
        return Ok(None);
    };
    Ok(stmt.query_row([target], |r| r.get(0)).optional()?)
}

fn redirect_target(conn: &Connection, public_id: &str) -> Result<Option<String>> {
    Ok(conn
        .prepare_cached("SELECT new_public_id FROM id_redirects WHERE old_public_id = ?1")?
        .query_row([public_id], |r| r.get(0))
        .optional()?)
}

/// Records that `old` now means `new`, after a merge. Redirects that pointed at `old`
/// are moved to the final target, and a redirect to an id that is itself redirected
/// points at that id's target, so every chain stays one step long (like the
/// `*_gid_redirect` tables of MusicBrainz).
pub fn add_redirect(tx: &Transaction<'_>, old: &str, new: &str) -> Result<()> {
    if old == new {
        return Ok(());
    }
    let mut target = redirect_target(tx, new)?.unwrap_or_else(|| new.to_owned());
    if target == old {
        // `new` had been merged into `old` and now the merge goes the other way:
        // `new` is live again, so its old redirect must go.
        tx.execute(
            "DELETE FROM id_redirects WHERE old_public_id = ?1",
            params![new],
        )?;
        target = new.to_owned();
    }
    tx.execute(
        "INSERT OR REPLACE INTO id_redirects (old_public_id, new_public_id, created_at)
         VALUES (?1, ?2, ?3)",
        params![old, target, now_ms()],
    )?;
    tx.execute(
        "UPDATE id_redirects SET new_public_id = ?2 WHERE new_public_id = ?1",
        params![old, target],
    )?;
    Ok(())
}
