//! Library-relative paths as stored in the database.
//!
//! The database always uses `/` as separator so that a database moved between operating
//! systems keeps working. These two functions are the only conversion point.

use std::path::{Component, Path, PathBuf};

/// Converts a path relative to the library root into its database form, with `/`
/// separators. Only normal components are kept (`.`, root and prefix are dropped).
/// Names that are not valid Unicode are stored lossily.
pub fn rel_path_to_db(rel: &Path) -> String {
    let mut out = String::new();
    for component in rel.components() {
        if let Component::Normal(name) = component {
            if !out.is_empty() {
                out.push('/');
            }
            out.push_str(&name.to_string_lossy());
        }
    }
    out
}

/// Converts a database path back into a native path relative to the library root.
pub fn rel_path_from_db(s: &str) -> PathBuf {
    s.split('/').filter(|part| !part.is_empty()).collect()
}
