//! Who can reach which library, and with which permission.
//!
//! A user sees no library until an admin grants one (`listen` or `manage`). Admins see
//! every library with `manage`.

use std::fmt;
use std::str::FromStr;

use rusqlite::{Connection, OptionalExtension, params};

use crate::ids::{LibraryId, UserId};
use crate::users::require_user;
use crate::{Error, Result};

/// Permission of a user on a library.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Permission {
    /// Browse, search and play.
    Listen,
    /// Everything `Listen` allows, plus adding and fixing music in the library.
    Manage,
}

impl Permission {
    /// The value stored in `library_access.permission`.
    pub fn as_str(self) -> &'static str {
        match self {
            Permission::Listen => "listen",
            Permission::Manage => "manage",
        }
    }
}

impl FromStr for Permission {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, String> {
        match s {
            "listen" => Ok(Permission::Listen),
            "manage" => Ok(Permission::Manage),
            other => Err(format!(
                "unknown permission {other:?}, expected listen or manage"
            )),
        }
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Gives `user` the permission `permission` on `library`, replacing any previous one.
pub fn grant(
    conn: &Connection,
    user: &UserId,
    library: LibraryId,
    permission: Permission,
) -> Result<()> {
    let key = require_user(conn, user)?;
    let exists: Option<i64> = conn
        .query_row("SELECT id FROM libraries WHERE id = ?1", [library.0], |r| {
            r.get(0)
        })
        .optional()?;
    if exists.is_none() {
        return Err(Error::LibraryNotFound(library));
    }
    conn.execute(
        "INSERT INTO library_access (user_id, library_id, permission) VALUES (?1, ?2, ?3)
         ON CONFLICT (user_id, library_id) DO UPDATE SET permission = excluded.permission",
        params![key, library.0, permission.as_str()],
    )?;
    Ok(())
}

/// Removes any permission of `user` on `library`. Returns whether there was one.
pub fn revoke(conn: &Connection, user: &UserId, library: LibraryId) -> Result<bool> {
    let key = require_user(conn, user)?;
    let deleted = conn.execute(
        "DELETE FROM library_access WHERE user_id = ?1 AND library_id = ?2",
        params![key, library.0],
    )?;
    Ok(deleted > 0)
}

/// What a user may do, loaded once per request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Access {
    /// The user.
    pub user: UserId,
    /// Login name.
    pub name: String,
    /// Whether the user administers the server.
    pub is_admin: bool,
    /// Whether the user may download original files.
    pub can_download: bool,
    /// Reachable libraries with their permission, by library identifier.
    pub libraries: Vec<(LibraryId, Permission)>,
}

impl Access {
    /// Loads the access of `user`. For an admin, `libraries` holds every library with
    /// [`Permission::Manage`].
    pub fn load(conn: &Connection, user: &UserId) -> Result<Access> {
        let key = require_user(conn, user)?;
        let (name, is_admin, can_download): (String, bool, bool) = conn.query_row(
            "SELECT name, is_admin, can_download FROM users WHERE id = ?1",
            [key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let libraries = if is_admin {
            conn.prepare_cached("SELECT id FROM libraries ORDER BY id")?
                .query_map([], |r| Ok((LibraryId(r.get(0)?), Permission::Manage)))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            let rows = conn
                .prepare_cached(
                    "SELECT library_id, permission FROM library_access
                     WHERE user_id = ?1 ORDER BY library_id",
                )?
                .query_map([key], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows.into_iter()
                .map(|(id, p)| {
                    // The CHECK constraint on the column allows only two values; if
                    // one slips in anyway, fail instead of guessing a permission.
                    let permission = p.parse().map_err(|_| Error::InvalidPermission(p))?;
                    Ok((LibraryId(id), permission))
                })
                .collect::<Result<Vec<_>>>()?
        };
        Ok(Access {
            user: user.clone(),
            name,
            is_admin,
            can_download,
            libraries,
        })
    }

    /// The libraries to look into: all reachable ones, or only `filter` if given. A
    /// filter on a library the user cannot reach gives an empty list, never an error,
    /// so the answer does not reveal whether that library exists.
    pub fn visible(&self, filter: Option<LibraryId>) -> Vec<LibraryId> {
        self.libraries
            .iter()
            .map(|&(id, _)| id)
            .filter(|&id| filter.is_none_or(|f| f == id))
            .collect()
    }

    /// The permission of the user on `library`, if any.
    pub fn permission(&self, library: LibraryId) -> Option<Permission> {
        self.libraries
            .iter()
            .find(|&&(id, _)| id == library)
            .map(|&(_, p)| p)
    }
}
