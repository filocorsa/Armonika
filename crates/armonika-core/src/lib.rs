//! Core library of the Armonika music server.

pub mod access;
pub mod artists;
pub mod db;
mod error;
pub mod ids;
pub mod libraries;
pub mod model;
pub mod paths;
pub mod secret;
pub mod text;
pub mod users;

pub use error::{Error, Result};

/// Version of the Armonika crates, taken from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
