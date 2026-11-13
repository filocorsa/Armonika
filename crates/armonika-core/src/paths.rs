//! Paths: library roots and library-relative paths as stored in the database.
//!
//! The database always uses `/` as separator in relative paths so that a database moved
//! between operating systems keeps working; [`rel_path_to_db`] and [`rel_path_from_db`]
//! are the only conversion point. Library roots are stored in canonical form and
//! checked by [`check_library_root`].

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

/// Why a directory cannot be the root of a library. The messages complete the sentence
/// "the directory ...".
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RootError {
    /// The directory is, contains or is inside the root of another library (given).
    #[error("overlaps the library root {}", .0.display())]
    Overlaps(PathBuf),
    /// The directory is or contains the data directory.
    #[error("contains the data directory")]
    ContainsDataDir,
    /// The directory is inside the data directory.
    #[error("is inside the data directory")]
    InsideDataDir,
    /// The path does not exist or is not a directory.
    #[error("is not an existing directory")]
    NotADirectory,
    /// The path is not valid Unicode, so it cannot be stored.
    #[error("is not valid Unicode")]
    NotUnicode,
}

/// The canonical form of an existing directory, as stored in `libraries.root`: absolute,
/// with links resolved and, on Windows, without the `\?\` prefix.
pub fn canonical_root(root: &Path) -> Result<PathBuf, RootError> {
    let canonical = dunce::canonicalize(root).map_err(|_| RootError::NotADirectory)?;
    if !canonical.is_dir() {
        return Err(RootError::NotADirectory);
    }
    if canonical.to_str().is_none() {
        return Err(RootError::NotUnicode);
    }
    Ok(canonical)
}

/// Checks that `new` can be the root of a library: an existing directory that neither
/// is, contains or is inside one of `other_roots`, nor is, contains or is inside
/// `data_dir`. Paths are compared in canonical form, without regard to case on Windows
/// and macOS, whose file systems usually ignore it.
pub fn check_library_root(
    new: &Path,
    other_roots: &[PathBuf],
    data_dir: &Path,
) -> Result<(), RootError> {
    let new_key = compare_key(&canonical_root(new)?);
    let data_key = compare_key(&canonical_or_absolute(data_dir));
    if data_key.starts_with(&new_key) {
        return Err(RootError::ContainsDataDir);
    }
    if new_key.starts_with(&data_key) {
        return Err(RootError::InsideDataDir);
    }
    for other in other_roots {
        let other_key = compare_key(&canonical_or_absolute(other));
        if new_key.starts_with(&other_key) || other_key.starts_with(&new_key) {
            return Err(RootError::Overlaps(other.clone()));
        }
    }
    Ok(())
}

/// Whether `a` and `b` name the same directory, by the rules of [`check_library_root`].
pub fn same_root(a: &Path, b: &Path) -> bool {
    compare_key(&canonical_or_absolute(a)) == compare_key(&canonical_or_absolute(b))
}

/// The canonical form of `path` if it exists, otherwise its absolute form (for example
/// a library root on a disk that is not mounted, or a data directory not created yet).
fn canonical_or_absolute(path: &Path) -> PathBuf {
    dunce::canonicalize(path)
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

/// The form in which two canonical paths are compared, component by component.
#[cfg(any(windows, target_os = "macos"))]
fn compare_key(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().to_lowercase())
}

/// The form in which two canonical paths are compared, component by component.
#[cfg(not(any(windows, target_os = "macos")))]
fn compare_key(path: &Path) -> PathBuf {
    path.to_path_buf()
}
