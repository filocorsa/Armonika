//! Libraries: named music directories.
//!
//! Each library has a root directory that never overlaps another library's root or the
//! data directory (see [`check_library_root`]). The root is stored in canonical form.
//! Removing a library deletes its tracks and their data from the database only: files
//! on disk are never touched.

use std::fmt;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, ErrorCode, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::db::now_ms;
use crate::ids::LibraryId;
use crate::paths::{canonical_root, check_library_root, same_root};
use crate::{Error, Result};

/// A library declared in the configuration file (`[[library]]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryConfig {
    /// Name of the library.
    pub name: String,
    /// Root directory.
    pub path: PathBuf,
}

/// A library as stored in the database.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Library {
    /// Public identifier.
    pub id: LibraryId,
    /// Name, unique.
    pub name: String,
    /// Root directory, canonical.
    pub root: PathBuf,
    /// Creation time, Unix milliseconds.
    pub created_at: i64,
}

/// What removing a library deletes, or would delete, from the database.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RemovalSummary {
    /// Tracks of the library, in any state.
    pub tracks: u64,
    /// Playlists that hold at least one of those tracks and lose those entries.
    pub playlists_touched: u64,
}

/// Something [`sync_from_config`] did or did not do, for the server log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncNote {
    /// The configured library did not exist and was created.
    Created {
        /// Name of the library.
        name: String,
        /// Its root as stored, canonical.
        root: PathBuf,
    },
    /// The library exists with another root: the database wins and the configured path
    /// is ignored.
    PathDiffers {
        /// Name of the library.
        name: String,
        /// Path in the configuration file.
        config_path: PathBuf,
        /// Root in the database, which is kept.
        db_root: PathBuf,
    },
    /// The configured library does not exist and cannot be created.
    NotCreated {
        /// Name of the library.
        name: String,
        /// Path in the configuration file.
        config_path: PathBuf,
        /// Why it cannot be created.
        reason: String,
    },
}

impl SyncNote {
    /// Whether the note reports a configured library that was not applied as written.
    pub fn is_warning(&self) -> bool {
        !matches!(self, SyncNote::Created { .. })
    }
}

impl fmt::Display for SyncNote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncNote::Created { name, root } => write!(
                f,
                "created library {name:?} at {} from the configuration",
                root.display()
            ),
            SyncNote::PathDiffers {
                name,
                config_path,
                db_root,
            } => write!(
                f,
                "library {name:?} is configured at {} but the database has {}; keeping {}. \
                 To move the library run `armonika library set-path`",
                config_path.display(),
                db_root.display(),
                db_root.display(),
            ),
            SyncNote::NotCreated {
                name,
                config_path,
                reason,
            } => write!(
                f,
                "library {name:?} at {} was not created: {reason}",
                config_path.display()
            ),
        }
    }
}

/// Creates a library and returns its identifier. `root` must pass
/// [`check_library_root`] against every other library and `data_dir`.
pub fn create(conn: &Connection, name: &str, root: &Path, data_dir: &Path) -> Result<LibraryId> {
    check_name(name)?;
    if by_name(conn, name)?.is_some() {
        return Err(Error::LibraryExists(name.to_owned()));
    }
    let canonical = checked_root(conn, None, root, data_dir)?;
    let inserted = conn.execute(
        "INSERT INTO libraries (name, root, created_at) VALUES (?1, ?2, ?3)",
        params![name, root_to_db(&canonical), now_ms()],
    );
    match inserted {
        Ok(_) => Ok(LibraryId(conn.last_insert_rowid())),
        Err(e) if e.sqlite_error_code() == Some(ErrorCode::ConstraintViolation) => {
            Err(Error::LibraryExists(name.to_owned()))
        }
        Err(e) => Err(e.into()),
    }
}

/// Renames a library.
pub fn rename(conn: &Connection, id: LibraryId, new_name: &str) -> Result<()> {
    check_name(new_name)?;
    require(conn, id)?;
    if by_name(conn, new_name)?.is_some_and(|l| l.id != id) {
        return Err(Error::LibraryExists(new_name.to_owned()));
    }
    match conn.execute(
        "UPDATE libraries SET name = ?2 WHERE id = ?1",
        params![id.0, new_name],
    ) {
        Ok(_) => Ok(()),
        Err(e) if e.sqlite_error_code() == Some(ErrorCode::ConstraintViolation) => {
            Err(Error::LibraryExists(new_name.to_owned()))
        }
        Err(e) => Err(e.into()),
    }
}

/// Moves a library to a new root, keeping its identifier and its tracks (the tracks
/// are matched again by the next scan). The new root is checked like in [`create`],
/// ignoring the library's own old root.
pub fn set_root(conn: &Connection, id: LibraryId, root: &Path, data_dir: &Path) -> Result<()> {
    require(conn, id)?;
    let canonical = checked_root(conn, Some(id), root, data_dir)?;
    conn.execute(
        "UPDATE libraries SET root = ?2 WHERE id = ?1",
        params![id.0, root_to_db(&canonical)],
    )?;
    Ok(())
}

/// What [`remove`] would delete, without deleting anything.
pub fn removal_summary(conn: &Connection, id: LibraryId) -> Result<RemovalSummary> {
    require(conn, id)?;
    let (tracks, playlists_touched): (i64, i64) = conn.query_row(
        "SELECT (SELECT COUNT(*) FROM tracks WHERE library_id = ?1),
                (SELECT COUNT(DISTINCT e.playlist_id) FROM playlist_entries e
                 JOIN tracks t ON t.id = e.track_id WHERE t.library_id = ?1)",
        [id.0],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(RemovalSummary {
        tracks: tracks.unsigned_abs(),
        playlists_touched: playlists_touched.unsigned_abs(),
    })
}

/// Removes a library from the database with its tracks and everything linked to them
/// (albums, annotations, history, playlist and queue entries). Files on disk are never
/// touched. Returns what was deleted.
pub fn remove(conn: &Connection, id: LibraryId) -> Result<RemovalSummary> {
    // A transaction of its own unless the caller already opened one.
    let tx = conn
        .is_autocommit()
        .then(|| conn.unchecked_transaction())
        .transpose()?;
    let summary = removal_summary(conn, id)?;
    conn.execute(
        "UPDATE playlists SET updated_at = ?2 WHERE id IN (
           SELECT e.playlist_id FROM playlist_entries e
           JOIN tracks t ON t.id = e.track_id WHERE t.library_id = ?1)",
        params![id.0, now_ms()],
    )?;
    // The foreign keys cascade to tracks, albums and every row that points at them.
    conn.execute("DELETE FROM libraries WHERE id = ?1", [id.0])?;
    if let Some(tx) = tx {
        tx.commit()?;
    }
    Ok(summary)
}

/// Every library, by name.
pub fn list(conn: &Connection) -> Result<Vec<Library>> {
    Ok(conn
        .prepare_cached(&format!("{LIBRARY_COLUMNS} ORDER BY name"))?
        .query_map([], library_from_row)?
        .collect::<rusqlite::Result<_>>()?)
}

/// The library called `name`.
pub fn by_name(conn: &Connection, name: &str) -> Result<Option<Library>> {
    Ok(conn
        .prepare_cached(&format!("{LIBRARY_COLUMNS} WHERE name = ?1"))?
        .query_row([name], library_from_row)
        .optional()?)
}

/// The library `id`.
pub fn get(conn: &Connection, id: LibraryId) -> Result<Option<Library>> {
    Ok(conn
        .prepare_cached(&format!("{LIBRARY_COLUMNS} WHERE id = ?1"))?
        .query_row([id.0], library_from_row)
        .optional()?)
}

/// Creates the configured libraries whose name does not exist yet, each reported by a
/// [`SyncNote::Created`]. For a library that exists the database wins: a different configured path is not applied and gives a
/// [`SyncNote::PathDiffers`]. A configured library that cannot be created (for example
/// a missing directory) gives a [`SyncNote::NotCreated`] and the others go on.
pub fn sync_from_config(
    conn: &Connection,
    configured: &[LibraryConfig],
    data_dir: &Path,
) -> Result<Vec<SyncNote>> {
    let mut notes = Vec::new();
    for lib in configured {
        if let Some(existing) = by_name(conn, &lib.name)? {
            if !same_root(&lib.path, &existing.root) {
                notes.push(SyncNote::PathDiffers {
                    name: lib.name.clone(),
                    config_path: lib.path.clone(),
                    db_root: existing.root,
                });
            }
            continue;
        }
        match create(conn, &lib.name, &lib.path, data_dir) {
            Ok(id) => notes.push(SyncNote::Created {
                name: lib.name.clone(),
                root: require_root(conn, id)?,
            }),
            Err(
                e @ (Error::LibraryRoot { .. }
                | Error::InvalidLibraryName(_)
                | Error::LibraryExists(_)),
            ) => notes.push(SyncNote::NotCreated {
                name: lib.name.clone(),
                config_path: lib.path.clone(),
                reason: e.to_string(),
            }),
            Err(e) => return Err(e),
        }
    }
    Ok(notes)
}

const LIBRARY_COLUMNS: &str = "SELECT id, name, root, created_at FROM libraries";

fn library_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Library> {
    Ok(Library {
        id: LibraryId(r.get(0)?),
        name: r.get(1)?,
        root: PathBuf::from(r.get::<_, String>(2)?),
        created_at: r.get(3)?,
    })
}

/// Checks `root` against the other libraries (all but `except`) and the data directory,
/// and returns its canonical form.
fn checked_root(
    conn: &Connection,
    except: Option<LibraryId>,
    root: &Path,
    data_dir: &Path,
) -> Result<PathBuf> {
    let root_error = |reason| Error::LibraryRoot {
        root: root.to_path_buf(),
        reason,
    };
    let others: Vec<PathBuf> = list(conn)?
        .into_iter()
        .filter(|l| Some(l.id) != except)
        .map(|l| l.root)
        .collect();
    check_library_root(root, &others, data_dir).map_err(root_error)?;
    canonical_root(root).map_err(root_error)
}

/// The canonical root as stored; [`canonical_root`] has already refused non-Unicode
/// paths.
fn root_to_db(canonical: &Path) -> String {
    canonical.to_string_lossy().into_owned()
}

/// The stored root of the library `id`.
fn require_root(conn: &Connection, id: LibraryId) -> Result<PathBuf> {
    let root: String = conn.query_row("SELECT root FROM libraries WHERE id = ?1", [id.0], |r| {
        r.get(0)
    })?;
    Ok(PathBuf::from(root))
}

fn require(conn: &Connection, id: LibraryId) -> Result<()> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM libraries WHERE id = ?1)",
        [id.0],
        |r| r.get(0),
    )?;
    if exists {
        Ok(())
    } else {
        Err(Error::LibraryNotFound(id))
    }
}

fn check_name(name: &str) -> Result<()> {
    if name.is_empty() || name.trim() != name || name.chars().any(char::is_control) {
        return Err(Error::InvalidLibraryName(name.to_owned()));
    }
    Ok(())
}
