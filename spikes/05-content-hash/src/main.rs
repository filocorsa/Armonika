//! Spike 5: content fingerprint of the compressed audio, tags excluded.
//!
//! Variant A hashes every compressed packet given by Symphonia's demuxer.
//! Variant B hashes 16 packets at the start, 16 around the middle and the last
//! 16 packets, plus the track length from the container header.
//!
//! Subcommands (run from this directory, see docs/spikes/05-content-hash.md):
//!   stability <messy_dir> <work_dir>   copy, edit tags, compare fingerprints
//!   cost <messy_dir> <work_dir>        time and bytes read per variant
//!   collision <work_dir>               sampled hash on files that differ only outside the samples
//!   probe <file>                       print fingerprints of one file

use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::AudioFile;
use lofty::flac::FlacFile;
use lofty::id3::v1::Id3v1Tag;
use lofty::id3::v2::Id3v2Tag;
use lofty::mp4::{Ilst, Mp4File};
use lofty::mpeg::MpegFile;
use lofty::ogg::tag::VorbisComments;
use lofty::ogg::{OpusFile, VorbisFile};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::tag::{Accessor, MergeTag, SplitTag, Tag, TagExt};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Timestamp;

const SAMPLE_PACKETS: usize = 16;

struct Counting {
    f: fs::File,
    bytes: Arc<AtomicU64>,
    len: u64,
}
impl Read for Counting {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        let n = self.f.read(b)?;
        self.bytes.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}
impl Seek for Counting {
    fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
        self.f.seek(p)
    }
}
impl MediaSource for Counting {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

#[derive(Clone, Debug)]
struct Fp {
    hash: String,
    packets: u64, // packets hashed
    bytes_read: u64,
    note: String,
}

fn open(path: &Path) -> Result<(Box<dyn FormatReader>, Arc<AtomicU64>), String> {
    let f = fs::File::open(path).map_err(|e| e.to_string())?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    let bytes = Arc::new(AtomicU64::new(0));
    let src = Counting { f, bytes: bytes.clone(), len };
    let mss = MediaSourceStream::new(Box::new(src), Default::default());
    let mut hint = Hint::new();
    if let Some(e) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(e);
    }
    let fmt = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| format!("probe: {e}"))?;
    Ok((fmt, bytes))
}

fn feed(h: &mut blake3::Hasher, data: &[u8]) {
    h.update(&(data.len() as u32).to_le_bytes());
    h.update(data);
}

/// Variant A: all packets of the first audio track.
fn fp_full(path: &Path) -> Result<Fp, String> {
    let (mut fmt, bytes) = open(path)?;
    let tid = fmt.default_track(TrackType::Audio).ok_or("no audio track")?.id;
    let mut h = blake3::Hasher::new();
    let mut n = 0u64;
    while let Some(p) = fmt.next_packet().map_err(|e| format!("packet: {e}"))? {
        if p.track_id != tid {
            continue;
        }
        feed(&mut h, &p.data);
        n += 1;
    }
    Ok(Fp { hash: h.finalize().to_hex()[..32].to_string(), packets: n, bytes_read: bytes.load(Ordering::Relaxed), note: String::new() })
}

fn read_n(fmt: &mut dyn FormatReader, tid: u32, n: usize, h: &mut blake3::Hasher, cnt: &mut u64) -> Result<(), String> {
    let mut left = n;
    while left > 0 {
        match fmt.next_packet().map_err(|e| format!("packet: {e}"))? {
            Some(p) if p.track_id == tid => {
                feed(h, &p.data);
                *cnt += 1;
                left -= 1;
            }
            Some(_) => {}
            None => break,
        }
    }
    Ok(())
}

/// Variant B: first, middle and last packets plus header track length.
fn fp_sampled(path: &Path) -> Result<Fp, String> {
    let (mut fmt, bytes) = open(path)?;
    let (tid, dur, nframes) = {
        let t = fmt.default_track(TrackType::Audio).ok_or("no audio track")?;
        (t.id, t.duration.map(|d| d.get()), t.num_frames)
    };
    let mut h = blake3::Hasher::new();
    let mut cnt = 0u64;
    let mut note = String::new();
    // Track length from the container header (not tag dependent).
    h.update(&dur.unwrap_or(u64::MAX).to_le_bytes());
    h.update(&nframes.unwrap_or(u64::MAX).to_le_bytes());
    read_n(&mut *fmt, tid, SAMPLE_PACKETS, &mut h, &mut cnt)?;
    match dur {
        Some(d) if d > 0 => {
            // Middle.
            fmt.seek(SeekMode::Coarse, SeekTo::Timestamp { ts: Timestamp::new((d / 2) as i64), track_id: tid })
                .map_err(|e| format!("seek mid: {e}"))?;
            read_n(&mut *fmt, tid, SAMPLE_PACKETS, &mut h, &mut cnt)?;
            // Last 16 packets: seek near the end, keep a ring of the final ones.
            let back = (d / 50).max(1); // about 2% of the track, enough for 16 packets in practice
            fmt.seek(SeekMode::Coarse, SeekTo::Timestamp { ts: Timestamp::new(d.saturating_sub(back) as i64), track_id: tid })
                .map_err(|e| format!("seek end: {e}"))?;
            let mut ring: std::collections::VecDeque<Box<[u8]>> = Default::default();
            while let Some(p) = fmt.next_packet().map_err(|e| format!("packet: {e}"))? {
                if p.track_id != tid {
                    continue;
                }
                ring.push_back(p.data);
                if ring.len() > SAMPLE_PACKETS {
                    ring.pop_front();
                }
            }
            for d in &ring {
                feed(&mut h, d);
                cnt += 1;
            }
        }
        _ => note = "no duration in header: first packets only".into(),
    }
    Ok(Fp { hash: h.finalize().to_hex()[..32].to_string(), packets: cnt, bytes_read: bytes.load(Ordering::Relaxed), note })
}

// ---------- tag editing (native lofty API, file level save) ----------

fn pic(data: &[u8]) -> Picture {
    Picture::unchecked(data.to_vec()).pic_type(PictureType::CoverFront).mime_type(MimeType::Jpeg).build()
}

fn split_merge<T>(native: T, path: &Path, opts: WriteOptions, m: &mut dyn FnMut(&mut Tag)) -> Result<(), String>
where
    T: SplitTag,
    <T::Remainder as MergeTag>::Merged: TagExt,
{
    let (rem, mut tag) = native.split_tag();
    m(&mut tag);
    rem.merge_tag(tag).save_to_path(path, opts).map_err(|e| e.to_string())
}

fn edit(path: &Path, opts: WriteOptions, m: &mut dyn FnMut(&mut Tag)) -> Result<(), String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_string();
    let mut fh = fs::File::open(path).map_err(|e| e.to_string())?;
    let po = ParseOptions::new();
    match ext.as_str() {
        "mp3" => {
            let file = MpegFile::read_from(&mut fh, po).map_err(|e| e.to_string())?;
            let t: Id3v2Tag = file.id3v2().cloned().unwrap_or_default();
            drop(fh);
            split_merge(t, path, opts, m)
        }
        "flac" => {
            let mut file = FlacFile::read_from(&mut fh, po).map_err(|e| e.to_string())?;
            let t: VorbisComments = file.vorbis_comments().cloned().unwrap_or_default();
            drop(fh);
            let (rem, mut tag) = t.split_tag();
            m(&mut tag);
            file.set_vorbis_comments(rem.merge_tag(tag));
            file.save_to_path(path, opts).map_err(|e| e.to_string())
        }
        "ogg" => {
            let file = VorbisFile::read_from(&mut fh, po).map_err(|e| e.to_string())?;
            let t = file.vorbis_comments().clone();
            drop(fh);
            split_merge(t, path, opts, m)
        }
        "opus" => {
            let file = OpusFile::read_from(&mut fh, po).map_err(|e| e.to_string())?;
            let t = file.vorbis_comments().clone();
            drop(fh);
            split_merge(t, path, opts, m)
        }
        "m4a" => {
            let file = Mp4File::read_from(&mut fh, po).map_err(|e| e.to_string())?;
            let t: Ilst = file.ilst().cloned().unwrap_or_default();
            drop(fh);
            split_merge(t, path, opts, m)
        }
        _ => Err(format!("unsupported ext {ext}")),
    }
}

fn strip_pictures(t: &mut Tag) {
    while t.picture_count() > 0 {
        t.remove_picture(0);
    }
}

fn copy_dir(src: &Path, dst: &Path) -> Vec<PathBuf> {
    fs::create_dir_all(dst).unwrap();
    let mut v = vec![];
    for e in fs::read_dir(src).unwrap() {
        let p = e.unwrap().path();
        if p.extension().and_then(|e| e.to_str()) == Some("json") {
            continue;
        }
        let d = dst.join(p.file_name().unwrap());
        fs::copy(&p, &d).unwrap();
        v.push(d);
    }
    v.sort();
    v
}

fn ext_of(p: &Path) -> String {
    p.extension().and_then(|e| e.to_str()).unwrap_or("").to_string()
}

type Action = Box<dyn Fn(&Path) -> Result<(), String>>;

fn stability(messy: &Path, work: &Path) {
    let _ = fs::remove_dir_all(work);
    let files = copy_dir(messy, &work.join("a"));
    let cover_small = fs::read(Path::new("../03-tags/work/cover_300k.jpg")).expect("cover_300k.jpg from spike 3");
    let cover_big = fs::read(Path::new("../03-tags/work/cover_5m.jpg")).expect("cover_5m.jpg from spike 3");
    let mut base: BTreeMap<String, (Fp, Fp)> = BTreeMap::new();
    let mut errors = 0;
    for p in &files {
        let n = p.file_name().unwrap().to_string_lossy().to_string();
        match (fp_full(p), fp_sampled(p)) {
            (Ok(a), Ok(b)) => {
                println!("BASE {n} A={} B={} packets_A={} packets_B={} {}", a.hash, b.hash, a.packets, b.packets, b.note);
                base.insert(n, (a, b));
            }
            (a, b) => {
                println!("BASE-ERR {n} A={:?} B={:?}", a.err(), b.err());
                errors += 1;
            }
        }
    }
    println!("base errors: {errors}");

    let cs = cover_small.clone();
    let cb = cover_big.clone();
    let mut cases: Vec<(&str, Action)> = vec![];
    cases.push(("title", Box::new(move |p| edit(p, WriteOptions::default(), &mut |t| t.set_title("Changed title".into())))));
    {
        let cs = cs.clone();
        cases.push(("cover_add_300k", Box::new(move |p| edit(p, WriteOptions::default(), &mut |t| t.push_picture(pic(&cs))))));
    }
    cases.push(("cover_add_5m", Box::new(move |p| edit(p, WriteOptions::default(), &mut |t| t.push_picture(pic(&cb))))));
    {
        let cs = cs.clone();
        cases.push((
            "cover_add_then_remove",
            Box::new(move |p| {
                edit(p, WriteOptions::default(), &mut |t| t.push_picture(pic(&cs)))?;
                edit(p, WriteOptions::default(), &mut |t| strip_pictures(t))
            }),
        ));
    }
    cases.push((
        "padding_64k_and_title",
        Box::new(move |p| edit(p, WriteOptions::default().preferred_padding(65536), &mut |t| t.set_title("Padded".into()))),
    ));
    cases.push((
        "padding_0_and_title",
        Box::new(move |p| edit(p, WriteOptions::default().preferred_padding(0), &mut |t| t.set_title("Tight".into()))),
    ));
    cases.push((
        "id3v1_add_then_remove",
        Box::new(move |p| {
            if ext_of(p) != "mp3" {
                return Err("skip".into());
            }
            let mut t = Id3v1Tag::default();
            t.set_title("v1 title".into());
            t.save_to_path(p, WriteOptions::default()).map_err(|e| e.to_string())?;
            lofty::tag::TagType::Id3v1.remove_from_path(p, WriteOptions::default()).map_err(|e| e.to_string())
        }),
    ));
    cases.push((
        "id3v1_added",
        Box::new(move |p| {
            if ext_of(p) != "mp3" {
                return Err("skip".into());
            }
            let mut t = Id3v1Tag::default();
            t.set_title("v1 title".into());
            t.save_to_path(p, WriteOptions::default()).map_err(|e| e.to_string())
        }),
    ));
    cases.push((
        "remove_all_tags",
        Box::new(move |p| match ext_of(p).as_str() {
            "mp3" => lofty::tag::TagType::Id3v2.remove_from_path(p, WriteOptions::default()).map_err(|e| e.to_string()),
            _ => edit(p, WriteOptions::default(), &mut |t| {
                t.clear();
            }),
        }),
    ));
    cases.push((
        "rename_and_move",
        Box::new(move |p| {
            let dir = p.parent().unwrap().join("moved").join("sub");
            fs::create_dir_all(&dir).unwrap();
            fs::rename(p, dir.join(format!("renamed.{}", ext_of(p)))).map_err(|e| e.to_string())
        }),
    ));

    let mut table: BTreeMap<(String, String), (u32, u32, u32)> = BTreeMap::new();
    let mut table_b: BTreeMap<(String, String), (u32, u32, u32)> = BTreeMap::new();
    let mut failures: Vec<String> = vec![];
    for (cname, act) in &cases {
        let cdir = work.join(format!("case_{cname}"));
        let _ = fs::remove_dir_all(&cdir);
        fs::create_dir_all(&cdir).unwrap();
        for p in &files {
            let n = p.file_name().unwrap().to_string_lossy().to_string();
            let Some((ba, bb)) = base.get(&n) else { continue };
            let q = cdir.join(&n);
            fs::copy(p, &q).unwrap();
            let fmt = ext_of(p);
            let key = (cname.to_string(), fmt.clone());
            if let Err(e) = act(&q) {
                if e != "skip" {
                    failures.push(format!("{cname} {n}: action error {e}"));
                }
                table.entry(key.clone()).or_default().2 += 1;
                table_b.entry(key).or_default().2 += 1;
                continue;
            }
            let final_path = if *cname == "rename_and_move" { cdir.join("moved").join("sub").join(format!("renamed.{fmt}")) } else { q.clone() };
            match (fp_full(&final_path), fp_sampled(&final_path)) {
                (Ok(a), Ok(b)) => {
                    let e = table.entry(key.clone()).or_default();
                    if a.hash == ba.hash {
                        e.0 += 1
                    } else {
                        e.1 += 1;
                        failures.push(format!("{cname} {n}: A differs"))
                    }
                    let e = table_b.entry(key).or_default();
                    if b.hash == bb.hash {
                        e.0 += 1
                    } else {
                        e.1 += 1;
                        failures.push(format!("{cname} {n}: B differs"))
                    }
                }
                (a, b) => failures.push(format!("{cname} {n}: fingerprint error A={:?} B={:?}", a.err(), b.err())),
            }
            if *cname == "rename_and_move" {
                let _ = fs::remove_dir_all(cdir.join("moved"));
            }
        }
    }
    println!("\nSTABILITY variant A (same/different/skipped files per case and format)");
    for ((c, f), (s, d, k)) in &table {
        println!("A | {c} | {f} | same={s} diff={d} skipped={k}");
    }
    println!("\nSTABILITY variant B");
    for ((c, f), (s, d, k)) in &table_b {
        println!("B | {c} | {f} | same={s} diff={d} skipped={k}");
    }
    println!("\nFAILURES: {}", failures.len());
    for f in &failures {
        println!("  {f}");
    }

    let gt: serde_json::Value = serde_json::from_str(&fs::read_to_string(messy.join("ground_truth.json")).unwrap()).unwrap();
    let mut by_a: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut by_b: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (n, (a, b)) in &base {
        by_a.entry(a.hash.clone()).or_default().push(n.clone());
        by_b.entry(b.hash.clone()).or_default().push(n.clone());
    }
    println!("\nDISTINCT hashes: A={} B={} of {} files", by_a.len(), by_b.len(), base.len());
    println!("Groups with more than one file (A):");
    for v in by_a.values().filter(|v| v.len() > 1) {
        println!("  {}", v.join(" == "));
    }
    println!("Groups with more than one file (B):");
    for v in by_b.values().filter(|v| v.len() > 1) {
        println!("  {}", v.join(" == "));
    }
    let arr = gt.as_array().unwrap();
    let (mut dup_equal, mut dup_total) = (0, 0);
    for e in arr {
        if let Some(orig) = e["is_duplicate_of"].as_str() {
            let f = e["file"].as_str().unwrap();
            let o = arr.iter().find(|x| x["source_id"] == orig && x["file"].as_str().unwrap().contains("-copy.")).unwrap();
            let of = o["file"].as_str().unwrap();
            dup_total += 1;
            if base[f].0.hash == base[of].0.hash {
                dup_equal += 1;
                println!("DUP-EQUAL (unexpected) {f} {of}");
            }
        }
    }
    println!("other-format duplicates: {dup_total}, with the same A hash as the original: {dup_equal}");
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

type Row = (f64, f64, u64, u64, u64);

fn cost(messy: &Path, work: &Path) {
    let files = copy_dir(messy, work);
    let total: u64 = files.iter().map(|p| fs::metadata(p).unwrap().len()).sum();
    println!("files={} total_bytes={} mean_MB={:.2}", files.len(), total, total as f64 / files.len() as f64 / 1e6);
    let mut res: BTreeMap<String, Vec<Row>> = BTreeMap::new();
    let mut warm: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut warm_b: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    // Pass 1: variant B first so that A does not warm the cache for B.
    for p in &files {
        let fmt = ext_of(p);
        let size = fs::metadata(p).unwrap().len();
        let t = Instant::now();
        let b = fp_sampled(p).unwrap();
        let msb = t.elapsed().as_secs_f64() * 1e3;
        let t = Instant::now();
        let a = fp_full(p).unwrap();
        let msa = t.elapsed().as_secs_f64() * 1e3;
        res.entry(fmt).or_default().push((msa, msb, a.bytes_read, b.bytes_read, size));
    }
    for _ in 0..3 {
        for p in &files {
            let fmt = ext_of(p);
            let t = Instant::now();
            fp_full(p).unwrap();
            warm.entry(fmt.clone()).or_default().push(t.elapsed().as_secs_f64() * 1e3);
            let t = Instant::now();
            fp_sampled(p).unwrap();
            warm_b.entry(fmt).or_default().push(t.elapsed().as_secs_f64() * 1e3);
        }
    }
    println!("\nfmt | n | mean size MB | pass1 A ms | pass1 B ms | A MB read | B MB read | warm A ms (median) | warm B ms (median)");
    let mut all = (0usize, 0f64, 0f64, 0f64, 0f64, 0f64);
    for (fmt, v) in &res {
        let n = v.len() as f64;
        let m = |f: &dyn Fn(&Row) -> f64| v.iter().map(f).sum::<f64>() / n;
        let (sa, sb, ba, bb, sz) = (m(&|x| x.0), m(&|x| x.1), m(&|x| x.2 as f64 / 1e6), m(&|x| x.3 as f64 / 1e6), m(&|x| x.4 as f64 / 1e6));
        let mut wa = warm[fmt].clone();
        let mut wb = warm_b[fmt].clone();
        println!("{fmt} | {} | {:.2} | {:.2} | {:.2} | {:.2} | {:.3} | {:.2} | {:.2}", v.len(), sz, sa, sb, ba, bb, median(&mut wa), median(&mut wb));
        all.0 += v.len();
        all.1 += sa * n;
        all.2 += sb * n;
        all.3 += ba * n;
        all.4 += bb * n;
        all.5 += sz * n;
    }
    let n = all.0 as f64;
    println!("ALL | {} | {:.2} | {:.2} | {:.2} | {:.2} | {:.3}", all.0, all.5 / n, all.1 / n, all.2 / n, all.3 / n, all.4 / n);
}

fn ffmpeg(args: &[&str]) {
    let o = Command::new("ffmpeg").args(["-y", "-v", "error"]).args(args).output().expect("ffmpeg");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

/// Two 60 s files, identical except for a tone burst between 10 s and 20 s
/// (outside the first 16 packets, the middle and the last 16 packets).
fn collision(work: &Path) {
    let _ = fs::remove_dir_all(work);
    fs::create_dir_all(work).unwrap();
    let w = |n: &str| work.join(n).to_string_lossy().to_string();
    for (name, freq) in [("a", "440"), ("b", "880")] {
        let src = format!("aevalsrc='if(between(t,10,20),0.5*sin(2*PI*{freq}*t),0)':s=44100:d=60");
        ffmpeg(&["-f", "lavfi", "-i", &src, "-c:a", "pcm_s16le", &w(&format!("{name}.wav"))]);
        ffmpeg(&["-i", &w(&format!("{name}.wav")), "-c:a", "libmp3lame", "-b:a", "128k", &w(&format!("{name}.mp3"))]);
        ffmpeg(&["-i", &w(&format!("{name}.wav")), "-c:a", "flac", &w(&format!("{name}.flac"))]);
        ffmpeg(&["-i", &w(&format!("{name}.wav")), "-c:a", "libvorbis", "-q:a", "4", &w(&format!("{name}.ogg"))]);
    }
    for ext in ["mp3", "flac", "ogg"] {
        let a = work.join(format!("a.{ext}"));
        let b = work.join(format!("b.{ext}"));
        let (fa, fb) = (fp_full(&a).unwrap(), fp_full(&b).unwrap());
        match (fp_sampled(&a), fp_sampled(&b)) {
            (Ok(sa), Ok(sb)) => println!(
                "{ext}: size a={} b={} | A equal={} | B equal={} (packets A={}, B sampled={}) | {}{}",
                fs::metadata(&a).unwrap().len(),
                fs::metadata(&b).unwrap().len(),
                fa.hash == fb.hash,
                sa.hash == sb.hash,
                fa.packets,
                sa.packets,
                sa.note,
                sb.note
            ),
            (x, y) => println!("{ext}: B error {:?} {:?}", x.err(), y.err()),
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(String::as_str) {
        Some("stability") => stability(Path::new(&a[2]), Path::new(&a[3])),
        Some("cost") => cost(Path::new(&a[2]), Path::new(&a[3])),
        Some("collision") => collision(Path::new(&a[2])),
        Some("probe") => {
            let p = Path::new(&a[2]);
            println!("A {:?}\nB {:?}", fp_full(p), fp_sampled(p));
        }
        _ => eprintln!("usage: stability|cost|collision|probe"),
    }
}
