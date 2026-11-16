//! The manifest written by `gen-big` and the comparison with what the server scanned.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

/// One manifest entry. Only the fields the harness uses are read.
#[derive(Debug, Clone, Deserialize)]
pub struct Entry {
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub album_artist: Option<String>,
}

pub fn load(path: &Path) -> anyhow::Result<Vec<Entry>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read manifest {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("bad manifest {}", path.display()))
}

/// A path with its folders in lowercase. Two folders that differ only by case are one
/// folder on NTFS and APFS (the files of both end up in the first one created), so
/// they must compare equal. The file name is kept as it is.
pub fn normalize_path(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((folders, file)) => format!("{}/{file}", folders.to_lowercase()),
        None => path.to_owned(),
    }
}

/// How the scanned tracks differ from the manifest.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Diff {
    /// In the manifest, not scanned (a few examples are kept).
    pub missing: Vec<String>,
    pub missing_count: usize,
    /// Scanned, not in the manifest (a few examples are kept).
    pub unexpected: Vec<String>,
    pub unexpected_count: usize,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.missing_count == 0 && self.unexpected_count == 0
    }
}

const EXAMPLES: usize = 5;

/// Compares the manifest paths with the scanned ones as multisets of normalised paths.
pub fn compare<'a>(
    manifest: impl IntoIterator<Item = &'a str>,
    scanned: impl IntoIterator<Item = &'a str>,
) -> Diff {
    let mut balance: BTreeMap<String, i64> = BTreeMap::new();
    for p in manifest {
        *balance.entry(normalize_path(p)).or_default() += 1;
    }
    for p in scanned {
        *balance.entry(normalize_path(p)).or_default() -= 1;
    }
    let mut diff = Diff::default();
    for (path, n) in balance {
        match n {
            0 => {}
            n if n > 0 => {
                diff.missing_count += n as usize;
                if diff.missing.len() < EXAMPLES {
                    diff.missing.push(path);
                }
            }
            n => {
                diff.unexpected_count += (-n) as usize;
                if diff.unexpected.len() < EXAMPLES {
                    diff.unexpected.push(path);
                }
            }
        }
    }
    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_are_lowercased_and_the_file_name_is_not() {
        assert_eq!(
            normalize_path("Alma Baker/Album (1999)/01 - A.mp3"),
            "alma baker/album (1999)/01 - A.mp3"
        );
        assert_eq!(normalize_path("Top.mp3"), "Top.mp3");
    }

    #[test]
    fn identical_sets_have_no_difference() {
        let d = compare(["A/x.mp3", "B/y.flac"], ["B/y.flac", "A/x.mp3"]);
        assert!(d.is_empty());
    }

    #[test]
    fn folders_that_differ_only_by_case_are_equal() {
        let d = compare(
            ["Ångström/a/1.mp3", "ÅNGSTRÖM/a/2.mp3"],
            ["Ångström/a/1.mp3", "Ångström/a/2.mp3"],
        );
        assert!(d.is_empty());
    }

    #[test]
    fn file_names_that_differ_by_case_are_different() {
        let d = compare(["A/x.mp3"], ["A/X.mp3"]);
        assert_eq!((d.missing_count, d.unexpected_count), (1, 1));
    }

    #[test]
    fn missing_and_unexpected_are_counted_with_examples() {
        let manifest: Vec<String> = (0..8).map(|i| format!("D/{i}.mp3")).collect();
        let d = compare(
            manifest.iter().map(String::as_str).chain(["D/dup.mp3"]),
            ["D/0.mp3", "D/extra.mp3"],
        );
        assert_eq!(d.missing_count, 8);
        assert_eq!(d.missing.len(), EXAMPLES);
        assert_eq!(d.unexpected, vec!["d/extra.mp3".to_owned()]);
        assert!(!d.is_empty());
    }

    #[test]
    fn a_path_scanned_twice_is_unexpected() {
        let d = compare(["A/x.mp3"], ["A/x.mp3", "a/x.mp3"]);
        assert_eq!((d.missing_count, d.unexpected_count), (0, 1));
    }

    #[test]
    fn the_manifest_of_gen_big_loads() {
        let json = r#"[{"path":"A/B/01.mp3","title":"T","artist":"X","album":"B","album_artist":"X","track":1,"disc":1,"year":2000}]"#;
        let entries: Vec<Entry> = serde_json::from_str(json).unwrap();
        assert_eq!(entries[0].path, "A/B/01.mp3");
        assert_eq!(entries[0].album.as_deref(), Some("B"));
    }
}
