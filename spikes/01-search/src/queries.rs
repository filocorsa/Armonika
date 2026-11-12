//! Deterministic target and query generation.

use crate::common::*;
use serde::{Deserialize, Serialize};

pub const SEED: u64 = 20261112;

#[derive(Serialize, Deserialize, Clone)]
pub struct Query {
    pub ty: String,       // prefix | typo | diacritics | multiword
    pub kind: String,     // track | album | artist
    pub text: String,     // what the user typed
    pub name: String,     // expected title / album / artist name
    pub artist: Option<String>, // expected artist (multiword only)
    pub chars: usize,     // typed length (prefix only)
    pub full: bool,       // prefix: complete name typed
}

#[derive(Serialize, Deserialize)]
pub struct QuerySet {
    pub seed: u64,
    pub targets: usize,
    pub queries: Vec<Query>,
}

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
}

fn foldable(name: &str) -> bool {
    fold(name) != name.to_lowercase()
}

fn sample<'a>(rng: &mut Rng, pool: &[&'a Doc], n: usize) -> Vec<&'a Doc> {
    let mut idx: Vec<usize> = (0..pool.len()).collect();
    let mut out = Vec::new();
    for _ in 0..n {
        let i = rng.below(idx.len());
        out.push(pool[idx.swap_remove(i)]);
    }
    out
}

fn make_typo(rng: &mut Rng, name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    // word boundaries
    let mut words: Vec<(usize, usize)> = Vec::new();
    let mut start = None;
    for (i, c) in chars.iter().enumerate() {
        if c.is_alphanumeric() {
            start.get_or_insert(i);
        } else if let Some(s) = start.take() {
            words.push((s, i));
        }
    }
    if let Some(s) = start {
        words.push((s, chars.len()));
    }
    let longest = words.iter().copied().max_by_key(|(s, e)| e - s).unwrap_or((0, chars.len()));
    let (s, e) = longest;
    let ascii_word = chars[s..e].iter().all(|c| c.is_ascii_alphabetic());
    let op = if ascii_word { rng.below(3) } else { 2 };
    let pos = s + rng.below(e - s);
    let mut out = chars.clone();
    let letter = (b'a' + rng.below(26) as u8) as char;
    match op {
        0 => {
            // substitution with a different letter
            let l = if out[pos].to_ascii_lowercase() == letter { 'q' } else { letter };
            out[pos] = l;
        }
        1 => out.insert(pos, letter),
        _ => {
            if e - s > 1 {
                out.remove(pos);
            }
        }
    }
    out.into_iter().collect()
}

pub fn build(data: &Data) -> QuerySet {
    let mut rng = Rng(SEED);
    // (kind, pool, total, foldable share)
    let plan: [(&str, &Vec<Doc>, usize, usize); 3] = [
        ("track", &data.tracks, 100, 30),
        ("album", &data.albums, 50, 15),
        ("artist", &data.artists, 50, 15),
    ];
    let mut queries = Vec::new();
    let mut targets = 0;
    for (kind, docs, total, n_fold) in plan {
        let mut seen = std::collections::HashSet::new();
        let (mut fold_pool, mut plain_pool) = (Vec::new(), Vec::new());
        for d in docs.iter() {
            if !seen.insert((d.a.clone(), d.b.clone())) {
                continue;
            }
            if foldable(&d.a) { fold_pool.push(d) } else { plain_pool.push(d) }
        }
        let mut chosen = sample(&mut rng, &fold_pool, n_fold);
        chosen.extend(sample(&mut rng, &plain_pool, total - n_fold));
        for d in chosen {
            targets += 1;
            let name = d.a.clone();
            let artist = if kind == "artist" { None } else { Some(d.b.clone()) };
            let mk = |ty: &str, text: String, chars: usize, full: bool, with_artist: bool| Query {
                ty: ty.into(),
                kind: kind.into(),
                text,
                name: name.clone(),
                artist: if with_artist { artist.clone() } else { None },
                chars,
                full,
            };
            let nchars = name.chars().count();
            for n in 3.min(nchars)..=nchars {
                let text: String = name.chars().take(n).collect();
                queries.push(mk("prefix", text, n, n == nchars, false));
            }
            queries.push(mk("typo", make_typo(&mut rng, &name), 0, false, false));
            if foldable(&name) {
                queries.push(mk("diacritics", fold(&name), 0, false, false));
            }
            let multi = match &artist {
                Some(a) => format!("{} {}", a, name),
                None => name.clone(),
            };
            queries.push(mk("multiword", multi, 0, false, true));
        }
    }
    QuerySet { seed: SEED, targets, queries }
}

pub fn path() -> std::path::PathBuf {
    data_dir().join("search-queries.json")
}

pub fn load() -> QuerySet {
    serde_json::from_str(&std::fs::read_to_string(path()).expect("run `queries` first")).unwrap()
}
