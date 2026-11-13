//! SQLite database: connections, pragmas and migrations.
//!
//! One writer connection behind a mutex and a small pool of reader connections. In WAL
//! mode readers never block the writer and always see the last committed transaction.

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use r2d2_sqlite::SqliteConnectionManager;
pub use rusqlite::Connection;
use rusqlite::{ErrorCode, TransactionBehavior};

use crate::{Error, Result};

/// Migrations in order; migration `i` (0-based) brings the schema to version `i + 1`.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_initial.sql"),
    include_str!("../migrations/0002_artist_links.sql"),
];

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
        // busy_timeout first: switching to WAL needs a lock another process may hold.
        configure(&writer)?;
        let mode = enable_wal(&writer)?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(Error::WalUnavailable(mode));
        }
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
        // A panic inside an earlier closure poisons the mutex. A `Transaction` it held
        // was rolled back when dropped, but one begun with SQL (or by a long batch that
        // never reached its commit) is still open: roll it back, so the next writer
        // does not commit half of someone else's work.
        let mut conn = self.inner.writer.lock().unwrap_or_else(|poisoned| {
            let conn = PoisonError::into_inner(poisoned);
            if !conn.is_autocommit() {
                // If even the rollback fails the connection is unusable anyway; the next
                // statement reports the error.
                let _ = conn.execute_batch("ROLLBACK");
            }
            self.inner.writer.clear_poison();
            conn
        });
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

/// Switches the database to WAL and returns the journal mode SQLite reports. Two
/// connections switching a new database at the same time can get SQLITE_BUSY at once,
/// without the busy handler (each holds a shared lock the other needs), so the switch
/// is retried until the busy timeout.
fn enable_wal(conn: &Connection) -> Result<String> {
    let deadline = Instant::now() + Duration::from_millis(BUSY_TIMEOUT_MS.into());
    loop {
        match conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0)) {
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == ErrorCode::DatabaseBusy && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            result => return Ok(result?),
        }
    }
}

/// Applies the migrations newer than `PRAGMA user_version`, each in its own IMMEDIATE
/// transaction that re-reads the version after taking the write lock: when two processes
/// open a new database at the same time, the second waits and then finds the migration
/// already applied instead of applying it again.
fn migrate(conn: &mut Connection) -> Result<()> {
    let latest = MIGRATIONS.len() as i64;
    loop {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if current > latest {
            return Err(Error::SchemaTooNew {
                found: current,
                latest,
            });
        }
        // A negative version would wrap to a huge count and silently skip every migration.
        let applied = usize::try_from(current).map_err(|_| Error::InvalidSchemaVersion(current))?;
        let Some(sql) = MIGRATIONS.get(applied) else {
            return Ok(());
        };
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", current + 1)?;
        tx.commit()?;
    }
}
