//! The "messy on purpose" library: take real Creative Commons recordings and damage their
//! metadata and layout the way real collections are damaged, keeping a ground truth.
//!
//! Rules of the build (see [`build_messy_library`]):
//! - each source is copied once into the output folder;
//! - mutating damages (`youtube_title`, `strip_tags`, `strip_cover`, `split_album`,
//!   `wrong_case`) are applied to that copy in manifest order;
//! - additive damages (`duplicate_other_format`, `compilation_copy`) produce extra files
//!   derived from the original source, never from the damaged copy;
//! - every output file has exactly one [`GroundTruth`] entry.

use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::tag::{Accessor, ItemKey, Tag, TagExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::big::AudioFormat;
use crate::names::sanitize_component;

/// One way of damaging a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Damage {
    /// Title becomes `"<artist> - <title> (Official Video) [HD]"`; album and track number
    /// are deleted.
    YoutubeTitle,
    /// All tags are deleted.
    StripTags,
    /// The cover is removed, text tags stay.
    StripCover,
    /// The album title becomes `"<album> (Deluxe)"`. The manifest decides which tracks of
    /// an edition get it (about half of them).
    SplitAlbum,
    /// A transcoded copy in the given format: same recording, same edition.
    DuplicateOtherFormat(AudioFormat),
    /// A copy that appears on a compilation: album `"Greatest Hits"`, album artist
    /// `"Various Artists"`. It is not a duplicate (different edition).
    CompilationCopy,
    /// Title and artist all lowercase.
    WrongCase,
}

impl Damage {
    /// Mutating damages change the single primary copy; the others add files.
    pub fn is_mutating(&self) -> bool {
        !matches!(
            self,
            Damage::DuplicateOtherFormat(_) | Damage::CompilationCopy
        )
    }

    fn label(&self) -> String {
        match self {
            Damage::YoutubeTitle => "youtube".into(),
            Damage::StripTags => "notags".into(),
            Damage::StripCover => "nocover".into(),
            Damage::SplitAlbum => "deluxe".into(),
            Damage::DuplicateOtherFormat(f) => f.extension().into(),
            Damage::CompilationCopy => "compilation".into(),
            Damage::WrongCase => "lower".into(),
        }
    }
}

/// What the library really contains, for scoring the later phases.
///
/// `file` is the name of the file inside the output folder (the output is flat).
/// A compilation copy keeps the source's MusicBrainz ids: it is the same recording even
/// though the edition on the file differs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroundTruth {
    pub file: PathBuf,
    pub source_id: String,
    pub recording_mbid: String,
    pub release_mbid: String,
    /// Set only for format duplicates: the id of the source they duplicate.
    pub is_duplicate_of: Option<String>,
}

// ---------------------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub id: String,
    pub source_url: String,
    pub sha256: String,
    pub license: String,
    pub attribution: String,
    pub recording_mbid: String,
    pub release_mbid: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DamageKind {
    YoutubeTitle,
    StripTags,
    StripCover,
    SplitAlbum,
    DuplicateOtherFormat,
    CompilationCopy,
    WrongCase,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct DamageEntry {
    source: String,
    kind: DamageKind,
    format: Option<String>,
}

/// A parsed and validated manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub sources: Vec<Source>,
    damages: Vec<(String, Damage)>,
}

impl Manifest {
    /// The damages of one source, in manifest order.
    pub fn damages_for(&self, source_id: &str) -> Vec<Damage> {
        self.damages
            .iter()
            .filter(|(id, _)| id == source_id)
            .map(|(_, d)| *d)
            .collect()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    #[serde(default)]
    source: Vec<Source>,
    #[serde(default)]
    damage: Vec<DamageEntry>,
}

/// Format names are file extensions: mp3, flac, ogg, opus, m4a, wav, aiff.
pub fn parse_format(name: &str) -> Option<AudioFormat> {
    Some(match name {
        "mp3" => AudioFormat::Mp3,
        "flac" => AudioFormat::Flac,
        "ogg" => AudioFormat::OggVorbis,
        "opus" => AudioFormat::Opus,
        "m4a" => AudioFormat::M4a,
        "wav" => AudioFormat::Wav,
        "aiff" => AudioFormat::Aiff,
        _ => return None,
    })
}

/// Parse the manifest TOML and check it: unique source ids, damages refer to known
/// sources, `format` present exactly for `duplicate_other_format` and valid.
pub fn parse_manifest(text: &str) -> Result<Manifest> {
    let raw: RawManifest = toml::from_str(text).context("invalid manifest")?;
    let mut ids = HashSet::new();
    for s in &raw.source {
        if !ids.insert(s.id.as_str()) {
            bail!("duplicate source id {:?}", s.id);
        }
    }
    let mut damages = Vec::with_capacity(raw.damage.len());
    for d in &raw.damage {
        if !ids.contains(d.source.as_str()) {
            bail!("damage refers to unknown source {:?}", d.source);
        }
        let damage = match (d.kind, &d.format) {
            (DamageKind::DuplicateOtherFormat, Some(f)) => {
                let format = parse_format(f)
                    .with_context(|| format!("unknown format {f:?} for source {:?}", d.source))?;
                Damage::DuplicateOtherFormat(format)
            }
            (DamageKind::DuplicateOtherFormat, None) => bail!(
                "duplicate_other_format needs a format (source {:?})",
                d.source
            ),
            (_, Some(_)) => bail!(
                "format is only allowed for duplicate_other_format (source {:?})",
                d.source
            ),
            (DamageKind::YoutubeTitle, None) => Damage::YoutubeTitle,
            (DamageKind::StripTags, None) => Damage::StripTags,
            (DamageKind::StripCover, None) => Damage::StripCover,
            (DamageKind::SplitAlbum, None) => Damage::SplitAlbum,
            (DamageKind::CompilationCopy, None) => Damage::CompilationCopy,
            (DamageKind::WrongCase, None) => Damage::WrongCase,
        };
        damages.push((d.source.clone(), damage));
    }
    Ok(Manifest {
        sources: raw.source,
        damages,
    })
}

/// File extension of a download URL (query and fragment ignored), lowercase.
pub fn source_extension(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?;
    let name = path.rsplit('/').next()?;
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

/// Lowercase hex SHA-256 of a file.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Fail with a clear message if the file does not have the expected SHA-256.
pub fn verify_sha256(path: &Path, expected: &str) -> Result<()> {
    let actual = sha256_file(path)?;
    if !actual.eq_ignore_ascii_case(expected.trim()) {
        bail!(
            "sha256 mismatch for {}: expected {}, got {}",
            path.display(),
            expected.trim(),
            actual
        );
    }
    Ok(())
}

/// The file in `sources_dir` whose name without extension is `id`.
pub fn find_source_file(sources_dir: &Path, id: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(sources_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && path.extension().is_some_and(|e| e != "part")
            && path.file_stem().is_some_and(|s| s == id)
        {
            return Some(path);
        }
    }
    None
}

// ---------------------------------------------------------------------------------------
// Damages
// ---------------------------------------------------------------------------------------

/// Pick a free `stem-label[-n].ext` name in `dir`.
fn unique_path(dir: &Path, stem: &str, label: &str, ext: &str) -> PathBuf {
    let stem = sanitize_component(stem);
    let mut n = 1u32;
    loop {
        let name = if n == 1 {
            format!("{stem}-{label}.{ext}")
        } else {
            format!("{stem}-{label}-{n}.{ext}")
        };
        let path = dir.join(name);
        if !path.exists() {
            return path;
        }
        n += 1;
    }
}

/// Copy `src` to a new file that must not exist yet.
fn copy_new(src: &Path, dst: &Path) -> Result<()> {
    let mut from = fs::File::open(src).with_context(|| format!("opening {}", src.display()))?;
    let mut to = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dst)
        .with_context(|| format!("creating {}", dst.display()))?;
    std::io::copy(&mut from, &mut to)?;
    to.flush()?;
    Ok(())
}

/// Apply `f` to every tag in the file (ID3v2 and ID3v1, RIFF INFO and ID3 chunk, and so
/// on), so that no secondary tag keeps the original values.
fn mutate_tags(path: &Path, f: impl Fn(&mut Tag)) -> Result<()> {
    let mut tagged =
        lofty::read_from_path(path).with_context(|| format!("reading {}", path.display()))?;
    if tagged.primary_tag().is_none() {
        let tag_type = tagged.primary_tag_type();
        tagged.insert_tag(Tag::new(tag_type));
    }
    let types: Vec<_> = tagged.tags().iter().map(|t| t.tag_type()).collect();
    for tag_type in types {
        let tag = tagged
            .tag_mut(tag_type)
            .context("tag disappeared while editing")?;
        f(tag);
        tag.save_to_path(path, WriteOptions::default())
            .with_context(|| format!("writing tags to {}", path.display()))?;
    }
    Ok(())
}

fn strip_all_tags(path: &Path) -> Result<()> {
    let tagged =
        lofty::read_from_path(path).with_context(|| format!("reading {}", path.display()))?;
    let types: Vec<_> = tagged.tags().iter().map(|t| t.tag_type()).collect();
    drop(tagged);
    for tag_type in types {
        tag_type
            .remove_from_path(path, WriteOptions::default())
            .with_context(|| format!("removing tags from {}", path.display()))?;
    }
    Ok(())
}

/// Encoder argument sets to try, best first. Some ffmpeg builds lack `libvorbis` or
/// `libopus`; the built-in experimental encoders (which need `-strict -2`) are the fallback.
fn ffmpeg_codec_args(format: AudioFormat) -> Vec<&'static [&'static str]> {
    match format {
        AudioFormat::Mp3 => vec![&["-c:a", "libmp3lame"]],
        AudioFormat::Flac => vec![&["-c:a", "flac"]],
        AudioFormat::OggVorbis => {
            vec![&["-c:a", "libvorbis"], &["-c:a", "vorbis", "-strict", "-2"]]
        }
        AudioFormat::Opus => vec![&["-c:a", "libopus"], &["-c:a", "opus", "-strict", "-2"]],
        AudioFormat::M4a => vec![&["-c:a", "aac"]],
        AudioFormat::Wav => vec![&["-c:a", "pcm_s16le"]],
        AudioFormat::Aiff => vec![&["-c:a", "pcm_s16be"]],
    }
}

/// Transcode with an external ffmpeg process (no shell). Tags are carried with
/// `-map_metadata 0`. The cover is kept (copied as an attached picture) when the target
/// is mp3, m4a or flac; for ogg, opus, wav and aiff it is dropped, because those
/// containers have no portable attached picture that ffmpeg writes.
fn transcode(src: &Path, dst: &Path, format: AudioFormat) -> Result<()> {
    let mut last_error = String::new();
    for codec in ffmpeg_codec_args(format) {
        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-v", "error", "-nostdin", "-n", "-i"])
            .arg(src)
            .args(["-map", "0:a:0", "-map_metadata", "0"]);
        if matches!(
            format,
            AudioFormat::Mp3 | AudioFormat::M4a | AudioFormat::Flac
        ) {
            cmd.args(["-map", "0:v:0?", "-c:v", "copy"]);
        }
        cmd.args(codec).arg(dst);
        let out = cmd
            .output()
            .context("running ffmpeg (is it installed and on PATH?)")?;
        if out.status.success() {
            return Ok(());
        }
        let _ = fs::remove_file(dst);
        last_error = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if !last_error.contains("Unknown encoder") && !last_error.contains("Encoder not found") {
            break;
        }
    }
    bail!(
        "ffmpeg failed for {} -> {}: {}",
        src.display(),
        dst.display(),
        last_error
    )
}

/// Apply one damage to `src` and return the new file(s), written into `dst_dir`.
///
/// `src` is never modified and existing files are never overwritten: the result gets a
/// name `<src stem>-<label>[-n].<ext>` that is free in `dst_dir`. To chain mutating
/// damages, apply the next one to the previous output.
pub fn apply_damage(src: &Path, dst_dir: &Path, damage: &Damage) -> Result<Vec<PathBuf>> {
    fs::create_dir_all(dst_dir).with_context(|| format!("creating {}", dst_dir.display()))?;
    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .context("source has no usable file name")?;
    let ext = src
        .extension()
        .and_then(|s| s.to_str())
        .context("source has no extension")?;
    if let Damage::DuplicateOtherFormat(format) = damage {
        let dst = unique_path(dst_dir, stem, &damage.label(), format.extension());
        transcode(src, &dst, *format)?;
        return Ok(vec![dst]);
    }
    let dst = unique_path(dst_dir, stem, &damage.label(), ext);
    copy_new(src, &dst)?;
    let result = match damage {
        Damage::YoutubeTitle => mutate_tags(&dst, |tag| {
            let artist = tag.artist().map(|s| s.into_owned()).unwrap_or_default();
            let title = tag.title().map(|s| s.into_owned()).unwrap_or_default();
            tag.set_title(format!("{artist} - {title} (Official Video) [HD]"));
            tag.remove_album();
            tag.remove_track();
            tag.remove_track_total();
        }),
        Damage::StripTags => strip_all_tags(&dst),
        Damage::StripCover => mutate_tags(&dst, |tag| {
            while !tag.pictures().is_empty() {
                tag.remove_picture(0);
            }
        }),
        Damage::SplitAlbum => mutate_tags(&dst, |tag| {
            if let Some(album) = tag.album().map(|s| s.into_owned()) {
                tag.set_album(format!("{album} (Deluxe)"));
            }
        }),
        Damage::CompilationCopy => mutate_tags(&dst, |tag| {
            tag.set_album("Greatest Hits".to_string());
            tag.insert_text(ItemKey::AlbumArtist, "Various Artists".to_string());
        }),
        Damage::WrongCase => mutate_tags(&dst, |tag| {
            if let Some(t) = tag.title().map(|s| s.to_lowercase()) {
                tag.set_title(t);
            }
            if let Some(a) = tag.artist().map(|s| s.to_lowercase()) {
                tag.set_artist(a);
            }
        }),
        Damage::DuplicateOtherFormat(_) => unreachable!("handled above"),
    };
    if let Err(e) = result {
        let _ = fs::remove_file(&dst);
        return Err(e);
    }
    Ok(vec![dst])
}

// ---------------------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------------------

/// Build the messy library in `out` from the sources in `sources_dir` (files named
/// `<id>.<ext>`) and write `out/ground_truth.json`. Returns the ground truth.
///
/// `out` must not contain a previous build. Source hashes are not checked here: the
/// `gen-messy` binary verifies them when it downloads or reuses a source.
pub fn build_messy_library(
    manifest: &Manifest,
    sources_dir: &Path,
    out: &Path,
) -> Result<Vec<GroundTruth>> {
    let truth_path = out.join("ground_truth.json");
    if out.exists() {
        let mut entries =
            fs::read_dir(out).with_context(|| format!("reading {}", out.display()))?;
        if entries.next().is_some() {
            bail!(
                "{} is not empty; remove it before generating again",
                out.display()
            );
        }
    }
    // Resolve all sources first so a missing file fails before anything is written.
    let mut files = Vec::with_capacity(manifest.sources.len());
    for s in &manifest.sources {
        let path = find_source_file(sources_dir, &s.id).with_context(|| {
            format!(
                "source {:?} not found in {} (expected {}.<ext>)",
                s.id,
                sources_dir.display(),
                s.id
            )
        })?;
        files.push(path);
    }
    fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let scratch = out.join(".scratch");
    fs::create_dir_all(&scratch)?;
    let result = build_into(manifest, &files, out, &scratch);
    let _ = fs::remove_dir_all(&scratch);
    let truth = result?;
    let json = serde_json::to_string_pretty(&truth)?;
    fs::write(&truth_path, json)?;
    Ok(truth)
}

fn build_into(
    manifest: &Manifest,
    files: &[PathBuf],
    out: &Path,
    scratch: &Path,
) -> Result<Vec<GroundTruth>> {
    let mut truth = Vec::new();
    for (source, original) in manifest.sources.iter().zip(files) {
        let damages = manifest.damages_for(&source.id);
        let ext = original
            .extension()
            .and_then(|e| e.to_str())
            .context("source has no extension")?;

        // Primary copy: all mutating damages in order, then one move into `out`.
        let mut current = original.clone();
        for d in damages.iter().filter(|d| d.is_mutating()) {
            let next = apply_damage(&current, scratch, d)
                .with_context(|| format!("source {:?}: {d:?}", source.id))?
                .remove(0);
            if current != *original {
                fs::remove_file(&current)?;
            }
            current = next;
        }
        let primary = unique_path(out, &source.id, "copy", ext);
        if current == *original {
            copy_new(original, &primary)?;
        } else {
            fs::rename(&current, &primary)?;
        }
        truth.push(entry(source, &primary, None)?);

        // Additive damages, derived from the original source.
        for d in damages.iter().filter(|d| !d.is_mutating()) {
            for file in apply_damage(original, out, d)
                .with_context(|| format!("source {:?}: {d:?}", source.id))?
            {
                let dup = matches!(d, Damage::DuplicateOtherFormat(_)).then(|| source.id.clone());
                truth.push(entry(source, &file, dup)?);
            }
        }
    }
    Ok(truth)
}

fn entry(source: &Source, file: &Path, is_duplicate_of: Option<String>) -> Result<GroundTruth> {
    Ok(GroundTruth {
        file: PathBuf::from(file.file_name().context("output without a file name")?),
        source_id: source.id.clone(),
        recording_mbid: source.recording_mbid.clone(),
        release_mbid: source.release_mbid.clone(),
        is_duplicate_of,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = r#"
[[source]]
id = "nin-ghosts-01"
source_url = "https://example.invalid/01.mp3?dl=1"
sha256 = "abc"
license = "CC BY-NC-SA 3.0"
attribution = "Nine Inch Nails, Ghosts I-IV"
recording_mbid = "rec"
release_mbid = "rel"
"#;

    #[test]
    fn parses_the_documented_format() {
        let text = format!(
            "{SOURCE}\n[[damage]]\nsource = \"nin-ghosts-01\"\nkind = \"youtube_title\"\n\n\
             [[damage]]\nsource = \"nin-ghosts-01\"\nkind = \"duplicate_other_format\"\nformat = \"mp3\"\n"
        );
        let m = parse_manifest(&text).unwrap();
        assert_eq!(m.sources.len(), 1);
        assert_eq!(m.sources[0].license, "CC BY-NC-SA 3.0");
        assert_eq!(
            m.damages_for("nin-ghosts-01"),
            vec![
                Damage::YoutubeTitle,
                Damage::DuplicateOtherFormat(AudioFormat::Mp3)
            ]
        );
    }

    #[test]
    fn comment_only_manifest_is_empty() {
        let m = parse_manifest("# just a header\n").unwrap();
        assert!(m.sources.is_empty());
    }

    #[test]
    fn unknown_kind_is_an_error() {
        let text =
            format!("{SOURCE}\n[[damage]]\nsource = \"nin-ghosts-01\"\nkind = \"explode\"\n");
        let err = format!("{:#}", parse_manifest(&text).unwrap_err());
        assert!(err.contains("explode") || err.contains("variant"), "{err}");
    }

    #[test]
    fn duplicate_other_format_needs_format() {
        let text = format!(
            "{SOURCE}\n[[damage]]\nsource = \"nin-ghosts-01\"\nkind = \"duplicate_other_format\"\n"
        );
        let err = format!("{:#}", parse_manifest(&text).unwrap_err());
        assert!(err.contains("needs a format"), "{err}");
    }

    #[test]
    fn rejects_bad_format_stray_format_unknown_source_and_duplicate_ids() {
        let bad_format = format!(
            "{SOURCE}\n[[damage]]\nsource = \"nin-ghosts-01\"\nkind = \"duplicate_other_format\"\nformat = \"wma\"\n"
        );
        assert!(parse_manifest(&bad_format).is_err());
        let stray = format!(
            "{SOURCE}\n[[damage]]\nsource = \"nin-ghosts-01\"\nkind = \"strip_tags\"\nformat = \"mp3\"\n"
        );
        assert!(parse_manifest(&stray).is_err());
        let unknown = format!("{SOURCE}\n[[damage]]\nsource = \"nope\"\nkind = \"strip_tags\"\n");
        assert!(parse_manifest(&unknown).is_err());
        assert!(parse_manifest(&format!("{SOURCE}{SOURCE}")).is_err());
    }

    #[test]
    fn source_extension_ignores_query_and_fragment() {
        assert_eq!(
            source_extension("https://x.invalid/a/b/01 Track.MP3?dl=1#x").as_deref(),
            Some("mp3")
        );
        assert_eq!(source_extension("https://x.invalid/a/b/"), None);
        assert_eq!(source_extension("https://x.invalid/noext"), None);
    }

    #[test]
    fn sha256_is_checked() {
        let dir = std::env::temp_dir().join(format!("armonika-sha-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("abc.txt");
        fs::write(&file, b"abc").unwrap();
        let good = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        verify_sha256(&file, good).unwrap();
        let err = verify_sha256(&file, "00").unwrap_err().to_string();
        assert!(err.contains("mismatch"), "{err}");
        fs::remove_dir_all(&dir).unwrap();
    }
}
