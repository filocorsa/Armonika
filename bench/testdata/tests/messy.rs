use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use armonika_testdata::AudioFormat;
use armonika_testdata::messy::{
    Damage, GroundTruth, apply_damage, build_messy_library, parse_manifest,
};
use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::picture::Picture;
use lofty::tag::{Accessor, ItemKey, Tag, TagExt};

fn make_cover(dir: &Path) -> PathBuf {
    let cover = dir.join("cover.jpg");
    let status = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=32x32",
        ])
        .args(["-frames:v", "1"])
        .arg(&cover)
        .status()
        .expect("ffmpeg must be installed");
    assert!(status.success());
    cover
}

/// A tagged source file made from a silent template, with a cover when asked.
fn make_source(dir: &Path, name: &str, format: AudioFormat, with_cover: bool) -> PathBuf {
    let path = dir.join(format!("{name}.{}", format.extension()));
    fs::copy(format.template_path(), &path).unwrap();
    let mut tagged = lofty::read_from_path(&path).unwrap();
    if tagged.primary_tag().is_none() {
        let t = tagged.primary_tag_type();
        tagged.insert_tag(Tag::new(t));
    }
    let tag = tagged.primary_tag_mut().unwrap();
    tag.set_title("Ghosts".to_string());
    tag.set_artist("Nine Inch Nails".to_string());
    tag.set_album("Ghosts I-IV".to_string());
    tag.insert_text(ItemKey::AlbumArtist, "Nine Inch Nails".to_string());
    tag.set_track(3);
    if with_cover {
        let cover = make_cover(dir);
        let mut f = fs::File::open(cover).unwrap();
        let pic = Picture::from_reader(&mut f).unwrap();
        tag.push_picture(pic);
    }
    tag.save_to_path(&path, WriteOptions::default()).unwrap();
    path
}

fn read_tag(path: &Path) -> Option<Tag> {
    let tagged = lofty::read_from_path(path).unwrap();
    tagged.primary_tag().or_else(|| tagged.first_tag()).cloned()
}

fn one(paths: Vec<PathBuf>) -> PathBuf {
    assert_eq!(paths.len(), 1, "{paths:?}");
    paths.into_iter().next().unwrap()
}

#[test]
fn youtube_title_sets_video_style_title_and_clears_album() {
    let dir = tempfile::tempdir().unwrap();
    let src = make_source(dir.path(), "a", AudioFormat::Mp3, false);
    let dst = tempfile::tempdir().unwrap();
    let out = one(apply_damage(&src, dst.path(), &Damage::YoutubeTitle).unwrap());
    let tag = read_tag(&out).unwrap();
    assert_eq!(
        tag.title().as_deref(),
        Some("Nine Inch Nails - Ghosts (Official Video) [HD]")
    );
    assert_eq!(tag.album(), None);
    assert_eq!(tag.track(), None);
}

#[test]
fn strip_tags_removes_all_tags() {
    for format in [AudioFormat::Mp3, AudioFormat::Flac, AudioFormat::M4a] {
        let dir = tempfile::tempdir().unwrap();
        let src = make_source(dir.path(), "a", format, false);
        let dst = tempfile::tempdir().unwrap();
        let out = one(apply_damage(&src, dst.path(), &Damage::StripTags).unwrap());
        let tagged = lofty::read_from_path(&out).unwrap();
        let empty = tagged.tags().iter().all(|t| t.is_empty());
        assert!(empty, "tags left in {}", out.display());
    }
}

#[test]
fn strip_cover_removes_pictures_but_keeps_text_tags() {
    for format in [AudioFormat::Mp3, AudioFormat::Flac, AudioFormat::M4a] {
        let dir = tempfile::tempdir().unwrap();
        let src = make_source(dir.path(), "a", format, true);
        assert!(!read_tag(&src).unwrap().pictures().is_empty());
        let dst = tempfile::tempdir().unwrap();
        let out = one(apply_damage(&src, dst.path(), &Damage::StripCover).unwrap());
        let tag = read_tag(&out).unwrap();
        assert!(tag.pictures().is_empty());
        assert_eq!(tag.title().as_deref(), Some("Ghosts"));
    }
}

#[test]
fn split_album_appends_deluxe_to_album_title() {
    let dir = tempfile::tempdir().unwrap();
    let src = make_source(dir.path(), "a", AudioFormat::Flac, false);
    let dst = tempfile::tempdir().unwrap();
    let out = one(apply_damage(&src, dst.path(), &Damage::SplitAlbum).unwrap());
    let tag = read_tag(&out).unwrap();
    assert_eq!(tag.album().as_deref(), Some("Ghosts I-IV (Deluxe)"));
    assert_eq!(tag.title().as_deref(), Some("Ghosts"));
}

#[test]
fn wrong_case_lowercases_title_and_artist() {
    let dir = tempfile::tempdir().unwrap();
    let src = make_source(dir.path(), "a", AudioFormat::Mp3, false);
    let dst = tempfile::tempdir().unwrap();
    let out = one(apply_damage(&src, dst.path(), &Damage::WrongCase).unwrap());
    let tag = read_tag(&out).unwrap();
    assert_eq!(tag.title().as_deref(), Some("ghosts"));
    assert_eq!(tag.artist().as_deref(), Some("nine inch nails"));
    assert_eq!(tag.album().as_deref(), Some("Ghosts I-IV"));
}

#[test]
fn compilation_copy_sets_greatest_hits_and_various_artists() {
    let dir = tempfile::tempdir().unwrap();
    let src = make_source(dir.path(), "a", AudioFormat::Mp3, false);
    let dst = tempfile::tempdir().unwrap();
    let out = one(apply_damage(&src, dst.path(), &Damage::CompilationCopy).unwrap());
    let tag = read_tag(&out).unwrap();
    assert_eq!(tag.album().as_deref(), Some("Greatest Hits"));
    assert_eq!(
        tag.get_string(ItemKey::AlbumArtist),
        Some("Various Artists")
    );
    assert_eq!(tag.title().as_deref(), Some("Ghosts"));
}

fn ffprobe_audio_codec(path: &Path) -> String {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "a:0"])
        .args(["-show_entries", "stream=codec_name", "-of", "csv=p=0"])
        .arg(path)
        .output()
        .expect("ffprobe must be installed");
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn decodes_cleanly(path: &Path) -> bool {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "null", "-"])
        .output()
        .expect("ffmpeg must be installed");
    out.status.success() && out.stderr.is_empty()
}

#[test]
fn duplicate_other_format_produces_decodable_file_in_target_format() {
    let dir = tempfile::tempdir().unwrap();
    let src = make_source(dir.path(), "a", AudioFormat::Flac, true);
    let expected = [
        (AudioFormat::Mp3, "mp3"),
        (AudioFormat::Flac, "flac"),
        (AudioFormat::OggVorbis, "vorbis"),
        (AudioFormat::Opus, "opus"),
        (AudioFormat::M4a, "aac"),
        (AudioFormat::Wav, "pcm_s16le"),
        (AudioFormat::Aiff, "pcm_s16be"),
    ];
    let dst = tempfile::tempdir().unwrap();
    for (format, codec) in expected {
        let out =
            one(apply_damage(&src, dst.path(), &Damage::DuplicateOtherFormat(format)).unwrap());
        assert_eq!(out.extension().unwrap(), format.extension());
        assert_eq!(ffprobe_audio_codec(&out), codec, "{}", out.display());
        assert!(decodes_cleanly(&out), "{}", out.display());
        if !matches!(format, AudioFormat::Wav | AudioFormat::Aiff) {
            let tag = read_tag(&out).unwrap_or_else(|| panic!("no tags in {}", out.display()));
            assert_eq!(tag.title().as_deref(), Some("Ghosts"), "{}", out.display());
            assert_eq!(tag.artist().as_deref(), Some("Nine Inch Nails"));
        }
        if matches!(
            format,
            AudioFormat::Mp3 | AudioFormat::Flac | AudioFormat::M4a
        ) {
            let tag = read_tag(&out).unwrap();
            assert!(
                !tag.pictures().is_empty(),
                "cover lost in {}",
                out.display()
            );
        }
    }
}

#[test]
fn apply_damage_never_modifies_source_and_never_overwrites() {
    let dir = tempfile::tempdir().unwrap();
    let src = make_source(dir.path(), "a", AudioFormat::Mp3, false);
    let before = fs::read(&src).unwrap();
    let dst = tempfile::tempdir().unwrap();
    let first = one(apply_damage(&src, dst.path(), &Damage::WrongCase).unwrap());
    let second = one(apply_damage(&src, dst.path(), &Damage::WrongCase).unwrap());
    assert_ne!(first, second);
    assert!(first.is_file() && second.is_file());
    assert_eq!(fs::read(&src).unwrap(), before);
    assert_eq!(first.parent().unwrap(), dst.path());
}

const MANIFEST: &str = r#"
[[source]]
id = "s1"
source_url = "https://example.invalid/s1.mp3"
sha256 = "00"
license = "CC BY 3.0"
attribution = "A"
recording_mbid = "rec-1"
release_mbid = "rel-1"

[[source]]
id = "s2"
source_url = "https://example.invalid/s2.flac"
sha256 = "00"
license = "CC BY 3.0"
attribution = "A"
recording_mbid = "rec-2"
release_mbid = "rel-1"

[[damage]]
source = "s1"
kind = "youtube_title"

[[damage]]
source = "s1"
kind = "wrong_case"

[[damage]]
source = "s1"
kind = "duplicate_other_format"
format = "opus"

[[damage]]
source = "s2"
kind = "compilation_copy"

[[damage]]
source = "s2"
kind = "strip_cover"
"#;

fn build_fixture() -> (tempfile::TempDir, tempfile::TempDir, Vec<GroundTruth>) {
    let sources = tempfile::tempdir().unwrap();
    make_source(sources.path(), "s1", AudioFormat::Mp3, true);
    make_source(sources.path(), "s2", AudioFormat::Flac, true);
    let out = tempfile::tempdir().unwrap();
    let manifest = parse_manifest(MANIFEST).unwrap();
    let truth = build_messy_library(&manifest, sources.path(), out.path()).unwrap();
    (sources, out, truth)
}

#[test]
fn ground_truth_lists_every_output_file() {
    let (_sources, out, truth) = build_fixture();
    let on_disk: HashSet<String> = fs::read_dir(out.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n != "ground_truth.json")
        .collect();
    let listed: HashSet<String> = truth
        .iter()
        .map(|t| t.file.to_string_lossy().into_owned())
        .collect();
    assert_eq!(on_disk, listed, "no extra and no missing files");
    assert_eq!(truth.len(), listed.len(), "one entry per file");
    // 2 primary copies + 1 format duplicate + 1 compilation copy.
    assert_eq!(truth.len(), 4);
    let json = fs::read_to_string(out.path().join("ground_truth.json")).unwrap();
    let parsed: Vec<GroundTruth> = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, truth);
}

#[test]
fn compilation_copy_is_not_marked_duplicate() {
    let (_s, out, truth) = build_fixture();
    let comp = truth
        .iter()
        .find(|t| {
            read_tag(&out.path().join(&t.file))
                .and_then(|tag| tag.album().map(|a| a.to_string()))
                .as_deref()
                == Some("Greatest Hits")
        })
        .expect("compilation copy present");
    assert_eq!(comp.is_duplicate_of, None);
    assert_eq!(comp.source_id, "s2");
}

#[test]
fn format_duplicate_points_to_source_and_primary_copies_do_not() {
    let (_s, out, truth) = build_fixture();
    let dups: Vec<&GroundTruth> = truth
        .iter()
        .filter(|t| t.is_duplicate_of.is_some())
        .collect();
    assert_eq!(dups.len(), 1);
    assert_eq!(dups[0].is_duplicate_of.as_deref(), Some("s1"));
    assert_eq!(dups[0].file.extension().unwrap(), "opus");
    assert_eq!(dups[0].recording_mbid, "rec-1");
    assert!(decodes_cleanly(&out.path().join(&dups[0].file)));
}

#[test]
fn mutating_damages_chain_in_manifest_order_on_one_copy() {
    let (_s, out, truth) = build_fixture();
    // s1: youtube_title then wrong_case, so a lowercase video style title.
    let primary = truth
        .iter()
        .find(|t| t.source_id == "s1" && t.is_duplicate_of.is_none())
        .unwrap();
    let tag = read_tag(&out.path().join(&primary.file)).unwrap();
    assert_eq!(
        tag.title().as_deref(),
        Some("nine inch nails - ghosts (official video) [hd]")
    );
    assert_eq!(tag.album(), None);
    // The duplicate is derived from the original source, not from the damaged copy.
    let dup = truth.iter().find(|t| t.is_duplicate_of.is_some()).unwrap();
    let dup_tag = read_tag(&out.path().join(&dup.file)).unwrap();
    assert_eq!(dup_tag.title().as_deref(), Some("Ghosts"));
}

#[test]
fn output_file_names_are_unique_and_windows_safe() {
    let (_s, _o, truth) = build_fixture();
    let names: HashSet<String> = truth
        .iter()
        .map(|t| t.file.to_string_lossy().to_lowercase())
        .collect();
    assert_eq!(names.len(), truth.len());
    for t in &truth {
        let n = t.file.to_string_lossy();
        assert!(
            !n.contains(['<', '>', ':', '"', '/', '\\', '|', '?', '*']),
            "{n}"
        );
    }
}

#[test]
fn missing_source_file_is_a_clear_error() {
    let sources = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let manifest = parse_manifest(MANIFEST).unwrap();
    let err = build_messy_library(&manifest, sources.path(), out.path()).unwrap_err();
    assert!(format!("{err:#}").contains("s1"), "{err:#}");
}

#[test]
fn damages_reach_secondary_id3v1_tag_too() {
    use lofty::tag::TagType;
    let dir = tempfile::tempdir().unwrap();
    let src = make_source(dir.path(), "a", AudioFormat::Mp3, false);
    // Add an ID3v1 tag carrying the true values next to the ID3v2 tag.
    let mut v1 = Tag::new(TagType::Id3v1);
    v1.set_title("Ghosts".to_string());
    v1.set_artist("Nine Inch Nails".to_string());
    v1.set_album("Ghosts I-IV".to_string());
    v1.set_track(3);
    v1.save_to_path(&src, WriteOptions::default()).unwrap();
    let tagged = lofty::read_from_path(&src).unwrap();
    assert!(tagged.tag(TagType::Id3v1).is_some() && tagged.tag(TagType::Id3v2).is_some());

    let dst = tempfile::tempdir().unwrap();
    let out = one(apply_damage(&src, dst.path(), &Damage::WrongCase).unwrap());
    let tagged = lofty::read_from_path(&out).unwrap();
    for t in [TagType::Id3v1, TagType::Id3v2] {
        let tag = tagged.tag(t).unwrap_or_else(|| panic!("{t:?} missing"));
        assert_eq!(tag.title().as_deref(), Some("ghosts"), "{t:?}");
        assert_eq!(tag.artist().as_deref(), Some("nine inch nails"), "{t:?}");
    }
    let out = one(apply_damage(&src, dst.path(), &Damage::CompilationCopy).unwrap());
    let tagged = lofty::read_from_path(&out).unwrap();
    assert_eq!(
        tagged.tag(TagType::Id3v1).unwrap().album().as_deref(),
        Some("Greatest Hits")
    );
}

#[test]
fn build_refuses_non_empty_output_folder() {
    let sources = tempfile::tempdir().unwrap();
    make_source(sources.path(), "s1", AudioFormat::Mp3, false);
    make_source(sources.path(), "s2", AudioFormat::Flac, false);
    let out = tempfile::tempdir().unwrap();
    fs::write(out.path().join("orphan.mp3"), b"x").unwrap();
    let manifest = parse_manifest(MANIFEST).unwrap();
    let err = build_messy_library(&manifest, sources.path(), out.path()).unwrap_err();
    assert!(format!("{err:#}").contains("not empty"), "{err:#}");
    assert!(!out.path().join("ground_truth.json").exists());
}
