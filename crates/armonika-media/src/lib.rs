//! Read-only access to audio files: tags, audio properties and the content hash.
//!
//! This crate never writes to a file; lofty and Symphonia are used only to read.

pub mod content_hash;
pub mod probe;
