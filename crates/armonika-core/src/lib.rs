//! Core library of the Armonika music server.

pub mod db;
mod error;
pub mod ids;
pub mod paths;
pub mod text;

pub use error::{Error, Result};

/// Version of the Armonika crates, taken from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
