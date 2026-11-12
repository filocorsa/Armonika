//! The large library generator: plan the metadata (pure), then copy templates and write tags.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::tag::{Accessor, ItemKey, Tag, TagExt};
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

use crate::names::{
    PathAllocator, PathParts, VARIOUS_ARTISTS, album_title, artist_name, build_relative_path,
    track_title,
};

/// Audio formats we have a silent template for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    Mp3,
    Flac,
    OggVorbis,
    Opus,
    M4a,
    Wav,
    Aiff,
}

impl AudioFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Flac => "flac",
            AudioFormat::OggVorbis => "ogg",
            AudioFormat::Opus => "opus",
            AudioFormat::M4a => "m4a",
            AudioFormat::Wav => "wav",
            AudioFormat::Aiff => "aiff",
        }
    }

    /// Location of the silent template in `bench/templates/`.
    pub fn template_path(&self) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("templates")
            .join(format!("silence.{}", self.extension()))
    }
}

/// What to generate. The same spec always gives the same library.
#[derive(Debug, Clone)]
pub struct BigLibrarySpec {
    pub tracks: usize,
    pub seed: u64,
    /// Formats are assigned to tracks in rotation.
    pub formats: Vec<AudioFormat>,
}

/// One generated file. `path` is relative to the output folder and is written to the
/// manifest with forward slashes, so manifests are identical on every platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedTrack {
    #[serde(with = "slash_path")]
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track: u32,
    pub disc: u32,
    pub year: u32,
}

mod slash_path {
    use std::path::{Path, PathBuf};

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(p: &Path, s: S) -> Result<S::Ok, S::Error> {
        let text = p
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        s.serialize_str(&text)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<PathBuf, D::Error> {
        let text = String::deserialize(d)?;
        Ok(text.split('/').collect())
    }
}

/// Fixed averages, for comparable benchmarks: tracks per edition, editions per artist.
const TRACKS_PER_EDITION: usize = 10;
const EDITIONS_PER_ARTIST: usize = 5;

fn feat_artist(rng: &mut ChaCha8Rng, main: &str, pool: &[String]) -> String {
    if rng.random_bool(0.03) {
        let other = &pool[rng.random_range(0..pool.len())];
        format!("{main} feat. {other}")
    } else {
        main.to_string()
    }
}

fn to_path(rel: &str) -> PathBuf {
    rel.split('/').collect()
}

/// Generate all metadata and relative paths without touching the disk.
/// [`generate_big_library`] writes exactly this plan.
pub fn plan_big_library(spec: &BigLibrarySpec) -> Vec<GeneratedTrack> {
    let mut out: Vec<GeneratedTrack> = Vec::with_capacity(spec.tracks);
    if spec.tracks == 0 || spec.formats.is_empty() {
        return out;
    }
    let mut rng = ChaCha8Rng::seed_from_u64(spec.seed);
    let artist_count = (spec.tracks / (TRACKS_PER_EDITION * EDITIONS_PER_ARTIST)).max(1);
    let pool: Vec<String> = (0..artist_count).map(|_| artist_name(&mut rng)).collect();
    let mut alloc = PathAllocator::new();

    while out.len() < spec.tracks {
        let remaining = spec.tracks - out.len();
        let slots = rng.random_range(6..=14usize).min(remaining);
        let various = rng.random_bool(0.02);
        let two_discs = rng.random_bool(0.05);
        let main_artist = pool[rng.random_range(0..pool.len())].clone();
        let album = album_title(&mut rng);
        let year = rng.random_range(1960..=2024u32);

        // Decide first which slots are album tracks and which are singles.
        let kinds: Vec<bool> = (0..slots).map(|_| rng.random_bool(0.01)).collect();
        let album_tracks = kinds.iter().filter(|single| !**single).count();
        let two_discs = two_discs && album_tracks >= 2;
        let disc1_len = album_tracks.div_ceil(2);
        let mut album_index = 0usize;

        for single in kinds {
            let idx = out.len();
            let ext = spec.formats[idx % spec.formats.len()].extension();
            if single {
                let who = pool[rng.random_range(0..pool.len())].clone();
                let artist = feat_artist(&mut rng, &who, &pool);
                let title = track_title(&mut rng);
                let rel = build_relative_path(&PathParts {
                    album_artist: None,
                    track_artist: &artist,
                    album: None,
                    year: 0,
                    disc: 1,
                    multi_disc: false,
                    track: 1,
                    title: &title,
                    extension: ext,
                });
                out.push(GeneratedTrack {
                    path: to_path(&alloc.allocate(rel)),
                    title,
                    artist,
                    album: None,
                    album_artist: None,
                    track: 1,
                    disc: 1,
                    year,
                });
                continue;
            }
            let (disc, track) = if two_discs && album_index >= disc1_len {
                (2, (album_index - disc1_len + 1) as u32)
            } else {
                (1, (album_index + 1) as u32)
            };
            album_index += 1;
            let base_artist = if various {
                pool[rng.random_range(0..pool.len())].clone()
            } else {
                main_artist.clone()
            };
            let artist = feat_artist(&mut rng, &base_artist, &pool);
            // The credit "X feat. Y" does not change the album artist.
            let album_artist = if various {
                VARIOUS_ARTISTS.to_string()
            } else {
                main_artist.clone()
            };
            let title = track_title(&mut rng);
            let rel = build_relative_path(&PathParts {
                album_artist: Some(&album_artist),
                track_artist: &artist,
                album: Some(&album),
                year,
                disc,
                multi_disc: two_discs,
                track,
                title: &title,
                extension: ext,
            });
            out.push(GeneratedTrack {
                path: to_path(&alloc.allocate(rel)),
                title,
                artist,
                album: Some(album.clone()),
                album_artist: Some(album_artist),
                track,
                disc,
                year,
            });
        }
    }
    out
}

fn write_tags(path: &Path, t: &GeneratedTrack) -> Result<()> {
    let mut tagged =
        lofty::read_from_path(path).with_context(|| format!("reading {}", path.display()))?;
    if tagged.primary_tag().is_none() {
        let tag_type = tagged.primary_tag_type();
        tagged.insert_tag(Tag::new(tag_type));
    }
    let tag = tagged
        .primary_tag_mut()
        .context("no tag available after insert")?;
    tag.set_title(t.title.clone());
    tag.set_artist(t.artist.clone());
    if let Some(album) = &t.album {
        tag.set_album(album.clone());
    }
    if let Some(aa) = &t.album_artist {
        tag.insert_text(ItemKey::AlbumArtist, aa.clone());
    }
    tag.set_track(t.track);
    tag.set_disk(t.disc);
    tag.insert_text(ItemKey::Year, t.year.to_string());
    tag.save_to_path(path, WriteOptions::default())
        .with_context(|| format!("writing tags to {}", path.display()))?;
    Ok(())
}

/// Generate the library under `out` and write `out/manifest.json`.
///
/// Returns the manifest entries (paths relative to `out`). Existing files are never
/// overwritten: if a target already exists the call fails.
pub fn generate_big_library(spec: &BigLibrarySpec, out: &Path) -> Result<Vec<GeneratedTrack>> {
    if spec.formats.is_empty() {
        bail!("at least one audio format is required");
    }
    let plan = plan_big_library(spec);
    fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let templates: Vec<Vec<u8>> = spec
        .formats
        .iter()
        .map(|f| {
            fs::read(f.template_path())
                .with_context(|| format!("reading template {}", f.template_path().display()))
        })
        .collect::<Result<_>>()?;
    for (i, t) in plan.iter().enumerate() {
        let full = out.join(&t.path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&full)
            .with_context(|| format!("creating {}", full.display()))?;
        f.write_all(&templates[i % templates.len()])?;
        drop(f);
        write_tags(&full, t)?;
    }
    let json = serde_json::to_string_pretty(&plan)?;
    fs::write(out.join("manifest.json"), json)?;
    Ok(plan)
}
