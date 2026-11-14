//! Administration commands that work on the database directly, without a running
//! server.

pub mod library;
pub mod track;
pub mod user;

use anyhow::Context;
use armonika_core::db::Db;
use armonika_core::secret::SecretBox;

use crate::config::Config;

/// Opens the database of `config.data_dir`, creating the directory if it is missing.
pub fn open_db(config: &Config) -> anyhow::Result<Db> {
    std::fs::create_dir_all(&config.data_dir)
        .with_context(|| format!("cannot create data dir {}", config.data_dir.display()))?;
    let db_path = config.db_path();
    Db::open(&db_path).with_context(|| format!("cannot open database {}", db_path.display()))
}

/// Opens the database and the secret key of `config.data_dir`, creating the
/// directory if it is missing.
pub fn open_store(config: &Config) -> anyhow::Result<(Db, SecretBox)> {
    let db = open_db(config)?;
    let secrets = SecretBox::load_or_create(&config.data_dir)?;
    Ok((db, secrets))
}
