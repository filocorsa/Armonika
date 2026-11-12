//! Synthetic 100k-track metadata with a realistic vocabulary (index-only, no audio files).
//! Pseudo-words from random syllables, Zipf-like word frequency, mostly unique titles,
//! about 10% of names with accents or CJK. Fixed seed.

use crate::common::*;
use std::collections::{BTreeMap, HashSet};

const SEED: u64 = 20261113;
const VOCAB: usize = 120_000;
const TRACKS: usize = 100_000;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

struct Zipf {
    cum: Vec<f64>,
}
impl Zipf {
    fn new(n: usize, s: f64) -> Self {
        let mut cum = Vec::with_capacity(n);
        let mut t = 0.0;
        for r in 1..=n {
            t += 1.0 / (r as f64).powf(s);
            cum.push(t);
        }
        Zipf { cum }
    }
    fn sample(&self, rng: &mut Rng) -> usize {
        let x = rng.unit() * self.cum[self.cum.len() - 1];
        self.cum.partition_point(|c| *c < x).min(self.cum.len() - 1)
    }
}

fn make_vocab(rng: &mut Rng) -> Vec<String> {
    let cons = [
        "b", "d", "f", "g", "k", "l", "m", "n", "p", "r", "s", "t", "v", "z", "br", "st", "tr", "ch", "sh", "gl", "cr",
        "fl", "pr", "th",
    ];
    let vows = ["a", "e", "i", "o", "u"];
    let accent = [
        ('a', "\u{e1}"),
        ('e', "\u{e9}"),
        ('o', "\u{f6}"),
        ('u', "\u{fc}"),
        ('i', "\u{ef}"),
        ('a', "\u{e5}"),
        ('o', "\u{f8}"),
        ('e', "\u{e8}"),
    ];
    let cjk: Vec<char> = (0..400).map(|i| char::from_u32(0x4E00 + i * 7).unwrap()).collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    while out.len() < VOCAB {
        let nsyl = 2 + rng.below(3);
        let mut w = String::new();
        for _ in 0..nsyl {
            w.push_str(cons[rng.below(cons.len())]);
            w.push_str(vows[rng.below(vows.len())]);
        }
        if rng.below(100) < 3 {
            w.push_str(["n", "r", "s", "l"][rng.below(4)]);
        }
        let r = rng.below(1000);
        if r < 40 {
            // accent one vowel (including o-slash)
            let (from, to) = accent[rng.below(accent.len())];
            if let Some(pos) = w.find(from) {
                w.replace_range(pos..pos + 1, to);
            }
        } else if r < 50 {
            // l-stroke
            if let Some(pos) = w.find('l') {
                w.replace_range(pos..pos + 1, "\u{142}");
            }
        } else if r < 70 {
            let n = 2 + rng.below(4);
            w = (0..n).map(|_| cjk[rng.below(cjk.len())]).collect();
        }
        if seen.insert(w.clone()) {
            out.push(w);
        }
    }
    out
}

fn capitalize(w: &str) -> String {
    let mut c = w.chars();
    match c.next() {
        Some(f) if f.is_alphabetic() => f.to_uppercase().collect::<String>() + c.as_str(),
        _ => w.to_string(),
    }
}

pub fn generate() {
    let mut rng = Rng(SEED);
    let vocab = make_vocab(&mut rng);
    let zipf = Zipf::new(VOCAB, 1.0);
    let phrase = |rng: &mut Rng, lo: usize, hi: usize| -> String {
        let n = lo + rng.below(hi - lo + 1);
        (0..n).map(|_| capitalize(&vocab[zipf.sample(rng)])).collect::<Vec<_>>().join(" ")
    };
    let n_artists = 5000;
    let mut artists = Vec::new();
    let mut seen = HashSet::new();
    while artists.len() < n_artists {
        let a = phrase(&mut rng, 1, 3);
        if seen.insert(a.clone()) {
            artists.push(a);
        }
    }
    // 1 to 4 albums per artist; albums are chosen for tracks with a Zipf-like popularity
    let mut albums: Vec<(usize, String)> = Vec::new();
    let mut aseen = HashSet::new();
    for i in 0..artists.len() {
        for _ in 0..(1 + rng.below(4)) {
            let al = phrase(&mut rng, 1, 4);
            if aseen.insert((i, al.clone())) {
                albums.push((i, al));
            }
        }
    }
    let az = Zipf::new(albums.len(), 0.6);
    let mut entries = Vec::with_capacity(TRACKS);
    let mut tseen = HashSet::new();
    for t in 0..TRACKS {
        let (ai, album) = &albums[az.sample(&mut rng)];
        let artist = &artists[*ai];
        // mostly unique titles: retry a few times, then accept a repeat
        let mut title = phrase(&mut rng, 1, 5);
        for _ in 0..3 {
            if tseen.insert(title.clone()) {
                break;
            }
            title = phrase(&mut rng, 1, 5);
        }
        let no_album = rng.below(100) < 5;
        entries.push(serde_json::json!({
            "path": format!("{artist}/{album}/{:02} - {title}.mp3", t % 99 + 1),
            "title": title,
            "artist": artist,
            "album": if no_album { serde_json::Value::Null } else { serde_json::json!(album) },
            "album_artist": if no_album { serde_json::Value::Null } else { serde_json::json!(artist) },
            "track": t % 99 + 1,
            "disc": 1,
            "year": 1960 + t % 60,
        }));
    }
    let dir = data_dir().join("realistic");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("manifest.json"), serde_json::to_string(&entries).unwrap()).unwrap();
    println!("generated {} tracks (seed {SEED}, vocabulary {VOCAB}, Zipf s=1)", entries.len());
}

/// Vocabulary statistics of the current dataset (tracks, titles, artists, albums).
pub fn stats() {
    let d = load_data();
    let mut toks: HashSet<String> = HashSet::new();
    let mut titles: BTreeMap<&str, usize> = BTreeMap::new();
    let mut non_ascii = 0usize;
    for t in &d.tracks {
        for f in [&t.a, &t.b, &t.c] {
            for x in tokens(f) {
                toks.insert(x);
            }
        }
        *titles.entry(t.a.as_str()).or_default() += 1;
        if t.a.chars().any(|c| !c.is_ascii()) {
            non_ascii += 1;
        }
    }
    let max_rep = titles.values().max().copied().unwrap_or(0);
    println!(
        "dataset {}: tracks {} albums {} artists {} distinct tokens {} distinct titles {} max title repeats {} titles with non-ASCII {:.1}%",
        dataset(),
        d.tracks.len(),
        d.albums.len(),
        d.artists.len(),
        toks.len(),
        titles.len(),
        max_rep,
        100.0 * non_ascii as f64 / d.tracks.len() as f64
    );
}
