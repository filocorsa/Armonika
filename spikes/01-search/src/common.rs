//! Shared types, text folding and the engine-agnostic rerank step.

use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

pub const CAND: usize = 50;
pub const TOP: usize = 10;

#[derive(Deserialize)]
struct ManifestEntry {
    title: String,
    artist: String,
    album: Option<String>,
    album_artist: Option<String>,
}

/// One result row. tracks: (title, artist, album); albums: (name, artist, ""); artists: (name, "", "").
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Doc {
    pub a: String,
    pub b: String,
    pub c: String,
}

#[derive(Default, Clone)]
pub struct Groups {
    pub tracks: Vec<Doc>,
    pub albums: Vec<Doc>,
    pub artists: Vec<Doc>,
}

pub struct Data {
    pub tracks: Vec<Doc>,
    pub albums: Vec<Doc>,
    pub artists: Vec<Doc>,
}

/// The engine-swappable interface: engines only produce candidates (native order).
pub trait SearchIndex {
    fn candidates(&self, query: &str) -> Groups;
}

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

pub fn data_dir() -> PathBuf {
    repo_root().join("bench").join("data")
}

pub fn load_data() -> Data {
    let path = data_dir().join("big-100k").join("manifest.json");
    let text = std::fs::read_to_string(&path).expect("read manifest");
    let entries: Vec<ManifestEntry> = serde_json::from_str(&text).expect("parse manifest");
    let mut tracks = Vec::with_capacity(entries.len());
    let mut albums = HashSet::new();
    let mut album_list = Vec::new();
    let mut artists = HashSet::new();
    let mut artist_list = Vec::new();
    for e in entries {
        let aa = e.album_artist.clone().unwrap_or_else(|| e.artist.clone());
        if let Some(al) = &e.album {
            if albums.insert((al.clone(), aa.clone())) {
                album_list.push(Doc { a: al.clone(), b: aa.clone(), c: String::new() });
            }
        }
        for name in [&e.artist, &aa] {
            if artists.insert(name.clone()) {
                artist_list.push(Doc { a: name.clone(), b: String::new(), c: String::new() });
            }
        }
        tracks.push(Doc { a: e.title, b: e.artist, c: e.album.unwrap_or_default() });
    }
    Data { tracks, albums: album_list, artists: artist_list }
}

fn latin(c: char) -> bool {
    (c as u32) < 0x250 || (0x1E00..0x1F00).contains(&(c as u32))
}

/// Lowercase and strip diacritics (also o-slash, l-stroke, sharp s, ligatures).
/// Only Latin letters are decomposed, so Hangul and CJK are left alone.
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if latin(c) {
            for d in std::iter::once(c).nfd() {
                if is_combining_mark(d) {
                    continue;
                }
                match d {
                    '\u{f8}' | '\u{d8}' => out.push('o'),
                    '\u{142}' | '\u{141}' => out.push('l'),
                    '\u{111}' | '\u{110}' => out.push('d'),
                    '\u{df}' => out.push_str("ss"),
                    '\u{e6}' | '\u{c6}' => out.push_str("ae"),
                    '\u{153}' | '\u{152}' => out.push_str("oe"),
                    '\u{131}' => out.push('i'),
                    _ => out.extend(d.to_lowercase()),
                }
            }
        } else {
            out.extend(c.to_lowercase());
        }
    }
    out
}

/// Folded tokens (used by the rerank and the Rust-side folding mode).
pub fn tokens(s: &str) -> Vec<String> {
    split_tokens(&fold(s))
}

pub fn split_tokens(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).map(String::from).collect()
}

fn lev(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let sub = prev[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            cur[j] = sub.min(prev[j] + 1).min(cur[j - 1] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

fn token_cost(qt: &[char], dt: &[char], is_last: bool) -> usize {
    if is_last {
        let n = dt.len().min(qt.len());
        lev(qt, &dt[..n]).min(lev(qt, dt))
    } else {
        lev(qt, dt)
    }
}

fn score(qtoks: &[Vec<char>], doc: &Doc, is_track: bool) -> f32 {
    let fields: [&str; 3] = [&doc.a, &doc.b, &doc.c];
    let dtoks: Vec<Vec<Vec<char>>> =
        fields.iter().map(|f| tokens(f).iter().map(|t| t.chars().collect()).collect()).collect();
    let total_doc_tokens: usize = dtoks.iter().map(Vec::len).sum();
    let mut cost = 0.0f32;
    for (i, qt) in qtoks.iter().enumerate() {
        let is_last = i + 1 == qtoks.len();
        let mut best = f32::MAX;
        for (fi, ft) in dtoks.iter().enumerate() {
            let w = if is_track && fi > 0 { 0.3 } else if !is_track && fi > 0 { 0.3 } else { 0.0 };
            for dt in ft {
                let d = token_cost(qt, dt, is_last).min(4) as f32 + w;
                if d < best {
                    best = d;
                }
            }
        }
        cost += if best == f32::MAX { 4.0 } else { best };
    }
    cost + 0.02 * total_doc_tokens as f32
}

fn rerank_group(query: &str, docs: &[Doc], is_track: bool) -> Vec<Doc> {
    let qtoks: Vec<Vec<char>> = tokens(query).iter().map(|t| t.chars().collect()).collect();
    if qtoks.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(f32, usize)> =
        docs.iter().enumerate().map(|(i, d)| (score(&qtoks, d, is_track), i)).collect();
    scored.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap().then(x.1.cmp(&y.1)));
    scored.into_iter().take(TOP).map(|(_, i)| docs[i].clone()).collect()
}

/// Engine-agnostic final ranking over the engine candidates.
pub fn search(idx: &dyn SearchIndex, query: &str) -> Groups {
    let c = idx.candidates(query);
    rerank(query, &c)
}

pub fn rerank(query: &str, c: &Groups) -> Groups {
    Groups {
        tracks: rerank_group(query, &c.tracks, true),
        albums: rerank_group(query, &c.albums, false),
        artists: rerank_group(query, &c.artists, false),
    }
}
