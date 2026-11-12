//! Spike 3: tag writing with lofty on all formats.
//!
//! For every format x case: prepare a copy of the silence template, write the
//! case with lofty, re-read with lofty, ffprobe and ffmpeg, then change only the
//! title and verify that everything else (tag values, unknown tag, cover bytes,
//! decoded audio) is unchanged. Results go to work/results.json and stdout.
//!
//! Run from this directory: cargo run --release
//! Needs ffmpeg and ffprobe on PATH, and work/cover_300k.jpg, work/cover_5m.jpg.

use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::flac::FlacFile;
use lofty::id3::v2::Frame;
use lofty::iff::aiff::AiffFile;
use lofty::iff::wav::WavFile;
use lofty::mp4::{AtomIdent, Mp4File};
use lofty::mpeg::MpegFile;
use lofty::ogg::{OggPictureStorage, OpusFile, VorbisFile};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::id3::v2::{ExtendedTextFrame, Id3v2Tag};
use lofty::iff::aiff::AiffTextChunks;
use lofty::iff::wav::RiffInfoList;
use lofty::mp4::{Atom, AtomData, Ilst};
use lofty::ogg::tag::VorbisComments;
use lofty::tag::{ItemKey, ItemValue, MergeTag, SplitTag, Tag, TagExt, TagItem, TagType};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const CASES: &[&str] = &[
    "basic",
    "mbids",
    "multi_artist",
    "unknown_tag",
    "unknown_freeform",
    "cover",
    "large_cover",
    "unicode",
];

struct Fmt {
    name: &'static str,
    ext: &'static str,
    tag_type: Option<TagType>, // None = lofty's primary tag type for the file
    id3v23: bool,
    ffmpeg_extra: &'static [&'static str],
}

const FORMATS: &[Fmt] = &[
    Fmt { name: "mp3", ext: "mp3", tag_type: None, id3v23: false, ffmpeg_extra: &[] },
    Fmt { name: "mp3-v23", ext: "mp3", tag_type: None, id3v23: true, ffmpeg_extra: &["-id3v2_version", "3"] },
    Fmt { name: "flac", ext: "flac", tag_type: None, id3v23: false, ffmpeg_extra: &[] },
    Fmt { name: "ogg", ext: "ogg", tag_type: None, id3v23: false, ffmpeg_extra: &[] },
    Fmt { name: "opus", ext: "opus", tag_type: None, id3v23: false, ffmpeg_extra: &[] },
    Fmt { name: "m4a", ext: "m4a", tag_type: None, id3v23: false, ffmpeg_extra: &[] },
    Fmt { name: "wav", ext: "wav", tag_type: None, id3v23: false, ffmpeg_extra: &["-write_id3v2", "1"] },
    Fmt { name: "wav-info", ext: "wav", tag_type: Some(TagType::RiffInfo), id3v23: false, ffmpeg_extra: &[] },
    Fmt { name: "aiff", ext: "aiff", tag_type: None, id3v23: false, ffmpeg_extra: &["-write_id3v2", "1"] },
    Fmt { name: "aiff-text", ext: "aiff", tag_type: Some(TagType::AiffText), id3v23: false, ffmpeg_extra: &[] },
];

const MBID_REC: &str = "11111111-1111-4111-8111-111111111111";
const MBID_TRACK: &str = "22222222-2222-4222-8222-222222222222";
const MBID_REL: &str = "33333333-3333-4333-8333-333333333333";
const MBID_RG: &str = "44444444-4444-4444-8444-444444444444";
const MBID_ART1: &str = "55555555-5555-4555-8555-555555555555";
const MBID_ART2: &str = "66666666-6666-4666-8666-666666666666";
const MBID_AART: &str = "77777777-7777-4777-8777-777777777777";
const MBID_WORK: &str = "88888888-8888-4888-8888-888888888888";

fn sha(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    h.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// Items expected for a case: (key, values). Pictures are handled separately.
fn expected_items(case: &str) -> Vec<(ItemKey, Vec<String>)> {
    let s = |v: &str| vec![v.to_string()];
    let basic = vec![
        (ItemKey::TrackTitle, s("Original title")),
        (ItemKey::TrackArtist, s("Test Artist")),
        (ItemKey::AlbumTitle, s("Test Album")),
        (ItemKey::AlbumArtist, s("Test Album Artist")),
        (ItemKey::TrackNumber, s("3")),
        (ItemKey::TrackTotal, s("12")),
        (ItemKey::DiscNumber, s("1")),
        (ItemKey::DiscTotal, s("2")),
        (ItemKey::RecordingDate, s("2021-05-14")),
    ];
    match case {
        "basic" | "unknown_tag" | "unknown_freeform" | "cover" | "large_cover" => basic,
        "mbids" => vec![
            (ItemKey::TrackTitle, s("Original title")),
            (ItemKey::MusicBrainzRecordingId, s(MBID_REC)),
            (ItemKey::MusicBrainzTrackId, s(MBID_TRACK)),
            (ItemKey::MusicBrainzReleaseId, s(MBID_REL)),
            (ItemKey::MusicBrainzReleaseGroupId, s(MBID_RG)),
            (ItemKey::MusicBrainzArtistId, vec![MBID_ART1.into(), MBID_ART2.into()]),
            (ItemKey::MusicBrainzReleaseArtistId, s(MBID_AART)),
            (ItemKey::MusicBrainzWorkId, s(MBID_WORK)),
        ],
        "multi_artist" => vec![
            (ItemKey::TrackTitle, s("Original title")),
            (ItemKey::TrackArtist, s("Artist One feat. Artist Two")),
            (ItemKey::TrackArtists, vec!["Artist One".into(), "Artist Two".into()]),
        ],
        "unicode" => vec![
            (ItemKey::TrackTitle, s("Caf\u{e9} \u{d1}and\u{fa} \u{65e5}\u{672c}\u{8a9e} \u{d55c}\u{ad6d}\u{c5b4} \u{1f3b5}")),
            (ItemKey::TrackArtist, s("\u{5742}\u{672c}\u{9f8d}\u{4e00} & Bj\u{f6}rk")),
            (ItemKey::AlbumTitle, s("\u{418}\u{437}\u{431}\u{440}\u{430}\u{43d}\u{43d}\u{43e}\u{435} \u{3a9}\u{3bc}\u{3ad}\u{3b3}\u{3b1}")),
        ],
        _ => unreachable!(),
    }
}

fn expected_picture(case: &str, root: &Path) -> Option<Vec<u8>> {
    match case {
        "cover" => Some(fs::read(root.join("work/cover_300k.jpg")).unwrap()),
        "large_cover" => Some(fs::read(root.join("work/cover_5m.jpg")).unwrap()),
        _ => None,
    }
}

fn run(cmd: &str, args: &[&str]) -> (bool, String, String) {
    let o = Command::new(cmd).args(args).output().expect("spawn");
    (
        o.status.success(),
        String::from_utf8_lossy(&o.stdout).to_string(),
        String::from_utf8_lossy(&o.stderr).to_string(),
    )
}

fn audio_md5(p: &Path) -> String {
    let (ok, out, err) = run("ffmpeg", &["-v", "error", "-i", p.to_str().unwrap(), "-map", "0:a:0", "-f", "md5", "-"]);
    if !ok || !err.trim().is_empty() {
        return format!("ERR:{}", err.trim());
    }
    out.trim().to_string()
}

fn decode_errors(p: &Path) -> String {
    let (ok, _, err) = run("ffmpeg", &["-v", "error", "-i", p.to_str().unwrap(), "-f", "null", "-"]);
    if ok && err.trim().is_empty() { String::new() } else { format!("exit_ok={ok} {}", err.trim()) }
}

/// Returns (ok, flat list of "scope:name" -> value as ffprobe sees them, raw json).
fn ffprobe(p: &Path) -> (bool, Vec<(String, String)>, Value) {
    let (ok, out, _) = run("ffprobe", &["-v", "error", "-show_format", "-show_streams", "-of", "json", p.to_str().unwrap()]);
    let v: Value = serde_json::from_str(&out).unwrap_or(Value::Null);
    let mut tags = vec![];
    let mut grab = |t: &Value, scope: &str| {
        if let Some(o) = t.as_object() {
            for (k, val) in o {
                tags.push((format!("{scope}:{k}"), val.as_str().unwrap_or("").to_string()));
            }
        }
    };
    grab(&v["format"]["tags"], "format");
    if let Some(ss) = v["streams"].as_array() {
        for (i, s) in ss.iter().enumerate() {
            grab(&s["tags"], &format!("stream{i}"));
        }
    }
    (ok, tags, v)
}

fn pic_line(p: &Picture) -> String {
    format!(
        "{:?}|{:?}|{}B|{}",
        p.pic_type(),
        p.mime_type().map(|m| m.as_str().to_string()),
        p.data().len(),
        sha(p.data())
    )
}

fn id3_lines(t: &lofty::id3::v2::Id3v2Tag, lines: &mut Vec<String>) {
    for fr in t {
        let id = fr.id().as_str().to_string();
        lines.push(match fr {
            Frame::UserText(x) => format!("TXXX:{}={}", x.description, x.content),
            Frame::Text(x) => format!("{id}={:?}", x.value),
            Frame::UniqueFileIdentifier(x) => {
                format!("UFID:{}={}", x.owner, String::from_utf8_lossy(&x.identifier))
            }
            Frame::Picture(x) => format!("APIC={}", pic_line(&x.picture)),
            other => {
                let d: String = format!("{other:?}").chars().take(120).collect();
                format!("{id}={d}")
            }
        });
    }
}

/// Native dump: one line per native item, as stored in the file.
fn native_dump(p: &Path, f: &Fmt, tt: TagType) -> Vec<String> {
    let mut fh = fs::File::open(p).unwrap();
    let po = ParseOptions::new();
    let mut lines: Vec<String> = vec![];
    match (f.ext, tt) {
        ("mp3", _) => {
            let file = MpegFile::read_from(&mut fh, po).unwrap();
            if let Some(t) = file.id3v2() {
                id3_lines(t, &mut lines);
            }
            if file.id3v1().is_some() {
                lines.push("ID3v1 present".into());
            }
            if file.ape().is_some() {
                lines.push("APE present".into());
            }
        }
        ("wav", TagType::Id3v2) => {
            let file = WavFile::read_from(&mut fh, po).unwrap();
            if let Some(t) = file.id3v2() {
                id3_lines(t, &mut lines);
            }
            if file.riff_info().is_some() {
                lines.push("RIFF INFO present".into());
            }
        }
        ("wav", _) => {
            let file = WavFile::read_from(&mut fh, po).unwrap();
            if let Some(t) = file.riff_info() {
                for (k, v) in t {
                    lines.push(format!("INFO:{k}={v}"));
                }
            }
            if file.id3v2().is_some() {
                lines.push("ID3v2 chunk present".into());
            }
        }
        ("aiff", TagType::Id3v2) => {
            let file = AiffFile::read_from(&mut fh, po).unwrap();
            if let Some(t) = file.id3v2() {
                id3_lines(t, &mut lines);
            }
            if file.text_chunks().is_some() {
                lines.push("AIFF text chunks present".into());
            }
        }
        ("aiff", _) => {
            let file = AiffFile::read_from(&mut fh, po).unwrap();
            if let Some(t) = file.text_chunks() {
                lines.push(format!("TEXT:name={:?}", t.name));
                lines.push(format!("TEXT:author={:?}", t.author));
                lines.push(format!("TEXT:copyright={:?}", t.copyright));
                lines.push(format!("TEXT:annotations={:?}", t.annotations));
                lines.push(format!("TEXT:comments={:?}", t.comments));
            }
            if file.id3v2().is_some() {
                lines.push("ID3v2 chunk present".into());
            }
        }
        ("flac", _) => {
            let file = FlacFile::read_from(&mut fh, po).unwrap();
            if let Some(t) = file.vorbis_comments() {
                lines.push(format!("VENDOR={}", t.vendor()));
                for (k, v) in t.items() {
                    lines.push(format!("{k}={v}"));
                }
                for (pic, _) in t.pictures() {
                    lines.push(format!("VC_PICTURE={}", pic_line(pic)));
                }
            }
            for (pic, _) in file.pictures() {
                lines.push(format!("PICTURE_BLOCK={}", pic_line(pic)));
            }
        }
        ("ogg", _) | ("opus", _) => {
            let vc = if f.ext == "ogg" {
                Some(VorbisFile::read_from(&mut fh, po).unwrap().vorbis_comments().clone())
            } else {
                Some(OpusFile::read_from(&mut fh, po).unwrap().vorbis_comments().clone())
            };
            if let Some(t) = vc {
                lines.push(format!("VENDOR={}", t.vendor()));
                for (k, v) in t.items() {
                    lines.push(format!("{k}={v}"));
                }
                for (pic, _) in t.pictures() {
                    lines.push(format!("PICTURE={}", pic_line(pic)));
                }
            }
        }
        ("m4a", _) => {
            let file = Mp4File::read_from(&mut fh, po).unwrap();
            if let Some(t) = file.ilst() {
                for atom in t {
                    let ident = match atom.ident() {
                        AtomIdent::Fourcc(b) => b.iter().map(|&c| c as char).collect::<String>(),
                        AtomIdent::Freeform { mean, name } => format!("----:{mean}:{name}"),
                    };
                    for d in atom.data() {
                        let val = match d {
                            lofty::mp4::AtomData::Picture(pic) => pic_line(pic),
                            other => format!("{other:?}"),
                        };
                        lines.push(format!("{ident}={val}"));
                    }
                }
            }
        }
        _ => unreachable!(),
    }
    lines
}

fn is_title_line(l: &str) -> bool {
    l.starts_with("TIT2=")
        || l.starts_with("TITLE=")
        || l.starts_with("\u{a9}nam=")
        || l.starts_with("INFO:INAM=")
        || l.starts_with("INFO:TITLE=")
        || l.starts_with("TEXT:name=")
}

fn write_opts(f: &Fmt) -> WriteOptions {
    let mut o = WriteOptions::default();
    if f.id3v23 {
        o = o.use_id3v23(true);
    }
    o
}

fn rank(s: &str) -> u8 {
    match s {
        "ok" | "n/a" => 0,
        "perso" => 1,
        _ => 2,
    }
}

fn worse(cur: &mut &'static str, to: &'static str) {
    if rank(to) > rank(cur) {
        *cur = to;
    }
}

/// Split the native tag into a generic Tag plus remainder, mutate the generic
/// part, merge back and write only that native tag.
fn split_merge<T>(native: T, path: &Path, opts: WriteOptions, mutate: &mut dyn FnMut(&mut Tag)) -> Result<(), String>
where
    T: SplitTag,
    <T::Remainder as MergeTag>::Merged: TagExt,
{
    let (rem, mut tag) = native.split_tag();
    mutate(&mut tag);
    rem.merge_tag(tag).save_to_path(path, opts).map_err(|e| e.to_string())
}

/// Edit through the native tag type of the file (split/merge), so that items
/// lofty has no generic key for are kept.
fn edit_split(path: &Path, f: &Fmt, tt: TagType, mutate: &mut dyn FnMut(&mut Tag)) -> Result<(), String> {
    let opts = write_opts(f);
    let mut fh = fs::File::open(path).map_err(|e| e.to_string())?;
    let po = ParseOptions::new();
    match (f.ext, tt) {
        ("mp3", _) => {
            let file = MpegFile::read_from(&mut fh, po).map_err(|e| e.to_string())?;
            let t: Id3v2Tag = file.id3v2().cloned().unwrap_or_default();
            drop(fh);
            split_merge(t, path, opts, mutate)
        }
        ("wav", TagType::Id3v2) => {
            let file = WavFile::read_from(&mut fh, po).map_err(|e| e.to_string())?;
            let t: Id3v2Tag = file.id3v2().cloned().unwrap_or_default();
            drop(fh);
            split_merge(t, path, opts, mutate)
        }
        ("wav", _) => {
            let file = WavFile::read_from(&mut fh, po).map_err(|e| e.to_string())?;
            let t: RiffInfoList = file.riff_info().cloned().unwrap_or_default();
            drop(fh);
            split_merge(t, path, opts, mutate)
        }
        ("aiff", TagType::Id3v2) => {
            let file = AiffFile::read_from(&mut fh, po).map_err(|e| e.to_string())?;
            let t: Id3v2Tag = file.id3v2().cloned().unwrap_or_default();
            drop(fh);
            split_merge(t, path, opts, mutate)
        }
        ("aiff", _) => {
            let file = AiffFile::read_from(&mut fh, po).map_err(|e| e.to_string())?;
            let t: AiffTextChunks = file.text_chunks().cloned().unwrap_or_default();
            drop(fh);
            split_merge(t, path, opts, mutate)
        }
        ("flac", _) => {
            let mut file = FlacFile::read_from(&mut fh, po).map_err(|e| e.to_string())?;
            let t: VorbisComments = file.vorbis_comments().cloned().unwrap_or_default();
            drop(fh);
            let (rem, mut tag) = t.split_tag();
            mutate(&mut tag);
            file.set_vorbis_comments(rem.merge_tag(tag));
            file.save_to_path(path, opts).map_err(|e| e.to_string())
        }
        ("ogg", _) => {
            let file = VorbisFile::read_from(&mut fh, po).map_err(|e| e.to_string())?;
            let t = file.vorbis_comments().clone();
            drop(fh);
            split_merge(t, path, opts, mutate)
        }
        ("opus", _) => {
            let file = OpusFile::read_from(&mut fh, po).map_err(|e| e.to_string())?;
            let t = file.vorbis_comments().clone();
            drop(fh);
            split_merge(t, path, opts, mutate)
        }
        ("m4a", _) => {
            let file = Mp4File::read_from(&mut fh, po).map_err(|e| e.to_string())?;
            let t: Ilst = file.ilst().cloned().unwrap_or_default();
            drop(fh);
            split_merge(t, path, opts, mutate)
        }
        _ => unreachable!(),
    }
}

/// Edit through the format-agnostic TaggedFile API (what most examples show).
fn edit_generic(path: &Path, f: &Fmt, tt: TagType, mutate: &mut dyn FnMut(&mut Tag)) -> Result<(), String> {
    let mut tf = lofty::read_from_path(path).map_err(|e| e.to_string())?;
    if tf.tag(tt).is_none() {
        tf.insert_tag(Tag::new(tt));
    }
    mutate(tf.tag_mut(tt).unwrap());
    tf.save_to_path(path, write_opts(f)).map_err(|e| e.to_string())
}

fn edit(path: &Path, f: &Fmt, tt: TagType, generic: bool, mutate: &mut dyn FnMut(&mut Tag)) -> Result<(), String> {
    if generic { edit_generic(path, f, tt, mutate) } else { edit_split(path, f, tt, mutate) }
}

fn main() {
    let root = std::env::current_dir().unwrap();
    let tpl_dir = root.join("../../bench/templates");
    let out_dir = root.join("work/out");
    let _ = fs::remove_dir_all(&out_dir);
    fs::create_dir_all(&out_dir).unwrap();
    let mut results: Vec<Value> = vec![];

    for f in FORMATS {
        let tpl = tpl_dir.join(format!("silence.{}", f.ext));
        let primary = lofty::read_from_path(&tpl).unwrap().primary_tag_type();
        let tt = f.tag_type.unwrap_or(primary);
        let mut modes: Vec<(&str, bool)> = vec![];
        for c in CASES {
            if *c == "unknown_freeform" && f.ext != "m4a" {
                continue;
            }
            modes.push((c, false));
            if c.starts_with("unknown") {
                modes.push((c, true));
            }
        }
        for (case, generic) in modes {
            let path: PathBuf = out_dir.join(format!("{}__{}{}.{}", f.name, case, if generic { "__generic" } else { "" }, f.ext));
            let mut notes: Vec<String> = vec![];
            let mut status: &'static str = "ok";

            // 1. prepare the starting file
            let mut unknown_present = false;
            if case == "unknown_tag" {
                let mut attempts: Vec<Vec<&str>> = vec![vec![]];
                if f.ext == "m4a" {
                    attempts.push(vec!["-movflags", "use_metadata_tags"]);
                }
                for extra in attempts {
                    let mut args: Vec<&str> =
                        vec!["-v", "error", "-y", "-i", tpl.to_str().unwrap(), "-map", "0:a", "-c", "copy"];
                    args.extend(f.ffmpeg_extra.iter().copied());
                    args.extend(extra.iter().copied());
                    args.extend(["-metadata", "ARMONIKA_TEST=keepme", path.to_str().unwrap()]);
                    let (ok, _, err) = run("ffmpeg", &args);
                    if !ok {
                        notes.push(format!("ffmpeg prepare failed: {}", err.trim()));
                        continue;
                    }
                    let (_, tags, _) = ffprobe(&path);
                    if tags.iter().any(|(k, v)| (k.to_uppercase().ends_with("ARMONIKA_TEST") || k.to_uppercase().ends_with("IZZZ")) && v == "keepme") {
                        unknown_present = true;
                        if !extra.is_empty() {
                            notes.push(format!("ffmpeg needed {extra:?} to write the custom tag"));
                        }
                        break;
                    }
                }
                if !unknown_present {
                    notes.push("ffmpeg could not store a custom tag in this container".into());
                    if !path.exists() {
                        fs::copy(&tpl, &path).unwrap();
                    }
                    if f.ext == "wav" && tt == TagType::Id3v2 {
                        let mut t = Id3v2Tag::default();
                        t.insert(Frame::UserText(ExtendedTextFrame::new(
                            lofty::TextEncoding::UTF8,
                            "ARMONIKA_TEST",
                            "keepme",
                        )));
                        t.save_to_path(&path, WriteOptions::default()).unwrap();
                        notes.push("custom TXXX written with lofty native Id3v2Tag instead".into());
                    } else if f.ext == "wav" && tt == TagType::RiffInfo {
                        let mut t = RiffInfoList::default();
                        t.insert("IZZZ".to_string(), "keepme".to_string());
                        t.save_to_path(&path, WriteOptions::default()).unwrap();
                        notes.push("custom INFO chunk IZZZ written with lofty native RiffInfoList instead".into());
                    }
                    let (_, tags, _) = ffprobe(&path);
                    unknown_present = tags.iter().any(|(k, v)| {
                        let k = k.to_uppercase();
                        (k.ends_with("ARMONIKA_TEST") || k.ends_with("IZZZ")) && v == "keepme"
                    });
                    if !unknown_present {
                        notes.push("custom tag not visible to ffprobe; unknown tag not checked".into());
                    }
                }
            } else if case == "unknown_freeform" {
                fs::copy(&tpl, &path).unwrap();
                let mut ilst = Ilst::default();
                ilst.insert(Atom::new(
                    AtomIdent::Freeform { mean: "com.apple.iTunes".into(), name: "ARMONIKA_TEST".into() },
                    AtomData::UTF8("keepme".into()),
                ));
                ilst.save_to_path(&path, WriteOptions::default()).unwrap();
                let (_, tags, _) = ffprobe(&path);
                unknown_present = tags.iter().any(|(k, v)| (k.to_uppercase().ends_with("ARMONIKA_TEST") || k.to_uppercase().ends_with("IZZZ")) && v == "keepme");
                notes.push("custom atom ----:com.apple.iTunes:ARMONIKA_TEST written with lofty native Ilst (simulates a foreign app)".into());
            } else {
                fs::copy(&tpl, &path).unwrap();
            }
            let md5_before = audio_md5(&path);
            let native_before = native_dump(&path, f, tt);

            // 2. write with lofty
            let mut rejected: Vec<String> = vec![];
            let exp = expected_items(case);
            let exp_pic = expected_picture(case, &root);
            let size_before = fs::metadata(&path).unwrap().len();
            let res = edit(&path, f, tt, generic, &mut |tag: &mut Tag| {
                for (key, vals) in &exp {
                    tag.remove_key(key.clone());
                    for v in vals {
                        if !tag.push(TagItem::new(key.clone(), ItemValue::Text(v.clone()))) {
                            rejected.push(format!("{key:?}"));
                        }
                    }
                }
                if let Some(data) = &exp_pic {
                    tag.push_picture(
                        Picture::unchecked(data.clone())
                            .pic_type(PictureType::CoverFront)
                            .mime_type(MimeType::Jpeg)
                            .build(),
                    );
                }
            });
            rejected.sort();
            rejected.dedup();
            if !rejected.is_empty() {
                notes.push(format!("lofty cannot store in {tt:?}: {}", rejected.join(", ")));
                worse(&mut status, "perso");
            }
            if exp_pic.is_some() && matches!(tt, TagType::RiffInfo | TagType::AiffText) {
                notes.push(format!("{tt:?} has no picture support"));
                worse(&mut status, "perso");
            }
            if let Err(e) = res {
                notes.push(format!("lofty save failed: {e}"));
                worse(&mut status, "corrotto");
                println!("{:10} {:16} {:8} {}", f.name, case, status, notes.join(" | "));
                results.push(json!({"format": f.name, "case": case, "generic_api": generic, "status": status, "notes": notes}));
                continue;
            }
            let size_after = fs::metadata(&path).unwrap().len();
            // everything the file had before must still be there (case-insensitive compare)
            {
                let after = native_dump(&path, f, tt);
                let low: Vec<String> = after.iter().map(|l| l.to_lowercase()).collect();
                for l in &native_before {
                    if is_title_line(l) {
                        continue;
                    }
                    if !low.contains(&l.to_lowercase()) {
                        if l.to_lowercase().starts_with("encoder=") {
                            notes.push(format!("minor: ENCODER comment folded into vendor string by lofty ({l})"));
                        } else {
                            notes.push(format!("pre-existing item lost on first write: {l}"));
                            worse(&mut status, "perso");
                        }
                    }
                }
            }

            // 3. re-read with lofty and compare
            let mut lofty_missing: Vec<String> = vec![];
            match lofty::read_from_path(&path) {
                Err(e) => {
                    notes.push(format!("lofty re-read failed: {e}"));
                    worse(&mut status, "corrotto");
                }
                Ok(tf) => match tf.tag(tt) {
                    None => {
                        notes.push("lofty re-read: tag container missing".into());
                        worse(&mut status, "perso");
                    }
                    Some(t) => {
                        for (key, vals) in &exp {
                            if rejected.contains(&format!("{key:?}")) {
                                continue;
                            }
                            let got: Vec<String> = t.get_strings(key.clone()).map(String::from).collect();
                            if &got != vals {
                                lofty_missing.push(format!("{key:?} expected {vals:?} got {got:?}"));
                            }
                        }
                        if let Some(data) = &exp_pic {
                            let pics = t.pictures();
                            if pics.len() != 1 || pics[0].data() != data.as_slice() {
                                lofty_missing.push(format!("picture mismatch (count {})", pics.len()));
                            }
                        }
                    }
                },
            }
            if !lofty_missing.is_empty() {
                notes.push(format!("lofty re-read mismatches: {}", lofty_missing.join("; ")));
                worse(&mut status, "perso");
            }

            // ffprobe + decode + audio hash
            let (probe_ok, ff_tags, ffv) = ffprobe(&path);
            if !probe_ok {
                notes.push("ffprobe failed".into());
                worse(&mut status, "corrotto");
            }
            let ff_title_val = ff_tags
                .iter()
                .find(|(k, _)| k.to_lowercase().ends_with(":title"))
                .map(|(_, v)| v.clone());
            let derr = decode_errors(&path);
            if !derr.is_empty() {
                notes.push(format!("ffmpeg decode: {derr}"));
                worse(&mut status, "corrotto");
            }
            let md5_after_write = audio_md5(&path);
            if md5_after_write != md5_before {
                notes.push("decoded audio changed after write".into());
                worse(&mut status, "corrotto");
            }
            let native_after_write = native_dump(&path, f, tt);
            let exp_title = exp.iter().find(|(k, _)| *k == ItemKey::TrackTitle).map(|(_, v)| v[0].clone());
            match &ff_title_val {
                None => {
                    notes.push("ffprobe shows no title".into());
                    worse(&mut status, "perso");
                }
                Some(v) if Some(v) != exp_title.as_ref() => {
                    notes.push(format!("ffprobe title differs: {v:?}"));
                    worse(&mut status, "perso");
                }
                _ => {}
            }
            if exp_pic.is_some() {
                let has_video = ffv["streams"]
                    .as_array()
                    .map(|s| s.iter().any(|x| x["codec_type"] == "video"))
                    .unwrap_or(false);
                if !has_video {
                    notes.push("ffprobe sees no attached picture stream".into());
                }
            }

            // 4. change only the title
            let mut changed_ok = true;
            if let Err(e) = edit(&path, f, tt, generic, &mut |t: &mut Tag| {
                t.remove_key(ItemKey::TrackTitle);
                t.push(TagItem::new(ItemKey::TrackTitle, ItemValue::Text("Changed title".into())));
            }) {
                notes.push(format!("title edit failed: {e}"));
                changed_ok = false;
            }
            let mut diffs: Vec<String> = vec![];
            let mut native_after = vec![];
            if changed_ok {
                native_after = native_dump(&path, f, tt);
                let strip = |v: &Vec<String>| -> Vec<String> {
                    v.iter().filter(|l| !is_title_line(l)).cloned().collect()
                };
                let a = strip(&native_after_write);
                let b = strip(&native_after);
                for l in &a {
                    if !b.contains(l) {
                        diffs.push(format!("lost after title edit: {l}"));
                    }
                }
                for l in &b {
                    if !a.contains(l) {
                        diffs.push(format!("changed after title edit: {l}"));
                    }
                }
                if !native_after.iter().any(|l| is_title_line(l) && l.contains("Changed title")) {
                    diffs.push("title not changed".into());
                }
                if audio_md5(&path) != md5_before {
                    diffs.push("decoded audio changed after title edit".into());
                }
                let d2 = decode_errors(&path);
                if !d2.is_empty() {
                    diffs.push(format!("decode after title edit: {d2}"));
                }
                let (pok, ft, _) = ffprobe(&path);
                if !pok {
                    diffs.push("ffprobe failed after title edit".into());
                }
                if unknown_present {
                    let ok = ft.iter().any(|(k, v)| (k.to_uppercase().ends_with("ARMONIKA_TEST") || k.to_uppercase().ends_with("IZZZ")) && v == "keepme");
                    if !ok {
                        diffs.push("unknown tag ARMONIKA_TEST lost (ffprobe)".into());
                    }
                }
            } else {
                worse(&mut status, "corrotto");
            }
            if !diffs.is_empty() {
                let bad = diffs
                    .iter()
                    .any(|d| d.contains("audio") || d.contains("decode") || d.contains("ffprobe failed"));
                notes.extend(diffs);
                worse(&mut status, if bad { "corrotto" } else { "perso" });
            }
            if case.starts_with("unknown") && !unknown_present && status == "ok" {
                status = "n/a";
            }

            results.push(json!({
                "format": f.name, "case": case, "generic_api": generic, "status": status, "tag_type": format!("{tt:?}"),
                "primary_tag_type": format!("{primary:?}"),
                "notes": notes, "size_before": size_before, "size_after": size_after,
                "audio_md5": md5_before,
                "ffprobe_tags": ff_tags.iter().map(|(k, v)| {
                    let v: String = v.chars().take(80).collect();
                    format!("{k}={v}")
                }).collect::<Vec<_>>(),
                "native_before": native_before,
                "native_after_write": native_after_write,
                "native_after_title_edit": native_after,
            }));
            println!("{:10} {:16} {:8} {}", f.name, format!("{case}{}", if generic { "/generic" } else { "" }), status, notes.join(" | "));
        }
    }
    fs::write(root.join("work/results.json"), serde_json::to_string_pretty(&results).unwrap()).unwrap();
}
