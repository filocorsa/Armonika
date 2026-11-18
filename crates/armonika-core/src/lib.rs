//! Core library of the Armonika music server.

pub mod access;
pub mod albums;
pub mod annotations;
pub mod artists;
pub mod db;
mod error;
pub mod fingerprints;
pub mod ids;
pub mod libraries;
pub mod model;
pub mod paths;
pub mod play_queue;
pub mod playlists;
pub mod search_source;
pub mod secret;
pub mod text;
pub mod track_files;
pub mod tracks;
pub mod users;

pub use error::{Error, Result};

/// Version of the Armonika crates, taken from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
