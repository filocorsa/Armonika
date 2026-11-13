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
    /// The secret key is not 32 bytes in base64. The message names where the key came
    /// from, never its value.
    #[error("the secret key from {from} is not 32 bytes encoded in base64")]
    InvalidSecretKey {
        /// Where the key came from.
        from: crate::secret::KeySource,
    },
    /// The secret key file could not be read or written.
    #[error("cannot access the secret key file {path}: {source}")]
    SecretKeyFile {
        /// Path of the key file.
        path: std::path::PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// The operating system random number generator failed.
    #[error("random number generator failed: {0}")]
    Random(String),
    /// Sealed data is truncated, was tampered with or was sealed with another key.
    #[error("sealed data cannot be decrypted (wrong secret key or corrupted data)")]
    Decrypt,
    /// Hashing a password failed.
    #[error("password hashing failed: {0}")]
    PasswordHash(String),
    /// A new password is shorter than the minimum.
    #[error("a password must be at least {min} characters long")]
    PasswordTooShort {
        /// Minimum number of characters.
        min: usize,
    },
    /// A user name is empty, has leading or trailing spaces or control characters.
    #[error("invalid user name {0:?}")]
    InvalidUserName(String),
    /// A user with this name (compared without case) already exists.
    #[error("a user named {0:?} already exists")]
    UserExists(String),
    /// No user has this identifier or name.
    #[error("user not found: {0}")]
    UserNotFound(String),
    /// The change would leave the server without an admin.
    #[error("the last admin cannot be deleted or demoted")]
    LastAdmin,
    /// The user already has an API key with this label.
    #[error("an API key labelled {0:?} already exists for this user")]
    ApiKeyLabelExists(String),
    /// The user has no API key with this label.
    #[error("no API key labelled {0:?} for this user")]
    ApiKeyNotFound(String),
    /// No library has this identifier.
    #[error("library not found: {0}")]
    LibraryNotFound(crate::ids::LibraryId),
}

/// Result type of the core library.
pub type Result<T> = std::result::Result<T, Error>;
