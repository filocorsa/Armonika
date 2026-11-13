//! SQLite database: connections, pragmas and migrations.
//!
//! One writer connection behind a mutex and a small pool of reader connections. In WAL
//! mode readers never block the writer and always see the last committed transaction.

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;

use crate::{Error, Result};

/// Migrations in order; migration `i` (0-based) brings the schema to version `i + 1`.
const MIGRATIONS: &[&str] = &[include_str!("../migrations/0001_initial.sql")];

/// Number of reader connections in the pool.
const READERS: u32 = 4;

/// Milliseconds SQLite waits on a locked database before giving up.
const BUSY_TIMEOUT_MS: u32 = 5000;

/// Handle to the Armonika database. Cheap to clone: clones share the same connections.
#[derive(Clone)]
pub struct Db {
    inner: Arc<Inner>,
}

struct Inner {
    writer: Mutex<Connection>,
    readers: r2d2::Pool<SqliteConnectionManager>,
}

impl std::fmt::Debug for Db {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Db").finish_non_exhaustive()
    }
}

impl Db {
    /// Opens (or creates) the database at `path`, sets the pragmas and applies any
    /// pending migration.
    pub fn open(path: &Path) -> Result<Db> {
        let mut writer = Connection::open(path)?;
        let mode: String =
            writer.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(Error::WalUnavailable(mode));
        }
        configure(&writer)?;
        migrate(&mut writer)?;

        let manager = SqliteConnectionManager::file(path).with_init(|c| {
            configure(c)?;
            c.pragma_update(None, "query_only", true)
        });
        let readers = r2d2::Pool::builder().max_size(READERS).build(manager)?;

        Ok(Db {
            inner: Arc::new(Inner {
                writer: Mutex::new(writer),
                readers,
            }),
        })
    }

    /// Runs `f` on a reader connection in a blocking task.
    pub async fn read<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let db = self.clone();
        tokio::task::spawn_blocking(move || db.read_blocking(f)).await?
    }

    /// Runs `f` on the writer connection in a blocking task.
    pub async fn write<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let db = self.clone();
        tokio::task::spawn_blocking(move || db.write_blocking(f)).await?
    }

    /// Runs `f` on a reader connection on the current thread. For the scanner and the CLI.
    pub fn read_blocking<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let conn = self.inner.readers.get()?;
        f(&conn)
    }

    /// Runs `f` on the writer connection on the current thread. For the scanner and the CLI.
    pub fn write_blocking<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        // A panic inside an earlier closure poisons the mutex; any open transaction
        // was rolled back when it was dropped, so the connection is still usable.
        let mut conn = self
            .inner
            .writer
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        f(&mut conn)
    }
}

/// Current time in Unix milliseconds, the unit of every time column.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Per-connection pragmas (journal mode is per database and set once by the writer).
fn configure(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.pragma_update(None, "busy_timeout", BUSY_TIMEOUT_MS)?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}

/// Applies the migrations newer than `PRAGMA user_version`, each in its own transaction.
fn migrate(conn: &mut Connection) -> Result<()> {
    let latest = MIGRATIONS.len() as i64;
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current > latest {
        return Err(Error::SchemaTooNew {
            found: current,
            latest,
        });
    }
    for (version, sql) in (1..).zip(MIGRATIONS).skip(current as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
    }
    Ok(())
}
