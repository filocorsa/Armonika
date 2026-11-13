//! Error type shared by the core library.

/// Errors returned by the core library.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A SQLite statement or connection failed.
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// No reader connection could be taken from the pool.
    #[error("database connection pool error: {0}")]
    Pool(#[from] r2d2::Error),
    /// The blocking task running a database closure panicked or was cancelled.
    #[error("database task failed: {0}")]
    Task(#[from] tokio::task::JoinError),
    /// SQLite refused to switch the database to WAL mode (for example on some network
    /// file systems); holds the journal mode it reported instead.
    #[error("could not enable WAL mode on the database, journal mode is {0}")]
    WalUnavailable(String),
    /// The database was written by a newer Armonika with a schema this build does not know.
    #[error("database schema version {found} is newer than the latest known version {latest}")]
    SchemaTooNew {
        /// Version found in `PRAGMA user_version`.
        found: i64,
        /// Latest migration shipped with this build.
        latest: i64,
    },
    /// A string is not a valid public identifier of the expected kind.
    #[error("invalid identifier {0:?}")]
    InvalidId(String),
}

/// Result type of the core library.
pub type Result<T> = std::result::Result<T, Error>;
