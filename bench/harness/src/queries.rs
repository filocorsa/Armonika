//! Deterministic query generation, with the scheme of spike 1: `prefix`, `typo`,
//! `diacritics` and `multiword` queries over track titles, albums and artists, from a
//! fixed seed.

use serde::Serialize;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::manifest::Entry;

/// The seed of spike 1, so the queries are the same on every run.
pub const SEED: u64 = 20261112;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum QueryType {
    Prefix,
    Typo,
    Diacritics,
    Multiword,
}

impl QueryType {
    pub const ALL: [QueryType; 4] = [
        QueryType::Prefix,
        QueryType::Typo,
        QueryType::Diacritics,
        QueryType::Multiword,
    ];

    pub fn name(self) -> &'static str {
        match self {
            QueryType::Prefix => "prefix",
            QueryType::Typo => "typo",
            QueryType::Diacritics => "diacritics",
            QueryType::Multiword => "multiword",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub ty: QueryType,
    pub text: String,
}

/// SplitMix64, the generator of spike 1.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn is_latin(c: char) -> bool {
    (c as u32) < 0x250 || (0x1E00..0x1F00).contains(&(c as u32))
}

/// Lowercase and strip diacritics (also o-slash, l-stroke, sharp s, ligatures).
/// Only Latin letters are decomposed, so Hangul and CJK are left alone.
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if !is_latin(c) {
            out.extend(c.to_lowercase());
            continue;
        }
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
    }
    out
}

/// Whether `text` has a Han, Kana or Hangul character.
pub fn has_cjk(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(u32::from(c),
            0x1100..=0x11FF | 0x2E80..=0x2FDF | 0x3040..=0x30FF | 0x3130..=0x318F
            | 0x31F0..=0x31FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7FF
            | 0xF900..=0xFAFF | 0xFF66..=0xFF9F | 0x20000..=0x323AF)
    })
}

/// A name that folding changes beyond lowercasing, so a folded query is a real test.
fn foldable(name: &str) -> bool {
    fold(name) != name.to_lowercase()
}

/// `name` with one typo in its longest word: a substitution, an insertion or a
/// deletion (only a deletion when the word is not plain ASCII letters).
pub fn make_typo(name: &str, rng: &mut Rng) -> String {
    let chars: Vec<char> = name.chars().collect();
    if chars.is_empty() {
        return String::new();
    }
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
    let (s, e) = words
        .iter()
        .copied()
        .max_by_key(|(s, e)| e - s)
        .unwrap_or((0, chars.len()));
    let ascii_word = chars[s..e].iter().all(char::is_ascii_alphabetic);
    let op = if ascii_word { rng.below(3) } else { 2 };
    let pos = s + rng.below(e - s);
    let letter = (b'a' + rng.below(26) as u8) as char;
    let mut out = chars;
    match op {
        0 => {
            out[pos] = if out[pos].to_ascii_lowercase() == letter {
                'q'
            } else {
                letter
            }
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

#[derive(Clone, Copy)]
enum Kind {
    Track,
    Album,
    Artist,
}

/// The name of `kind` for an entry, and the artist to put before it in a multiword query.
fn target(e: &Entry, kind: Kind) -> (String, Option<String>) {
    match kind {
        Kind::Track => (e.title.clone(), Some(e.artist.clone())),
        Kind::Album => match &e.album {
            Some(album) if !album.is_empty() => (
                album.clone(),
                Some(e.album_artist.clone().unwrap_or_else(|| e.artist.clone())),
            ),
            _ => (e.title.clone(), Some(e.artist.clone())),
        },
        Kind::Artist => (
            e.album_artist.clone().unwrap_or_else(|| e.artist.clone()),
            None,
        ),
    }
}

/// `count` queries, the four types in rotation, over track titles, albums and artists
/// (in the 2:1:1 shares of spike 1). The same manifest, count and seed always give the
/// same queries. An empty manifest gives none.
pub fn build(entries: &[Entry], count: usize, seed: u64) -> Vec<Query> {
    let entries: Vec<&Entry> = entries.iter().filter(|e| !e.title.is_empty()).collect();
    if entries.is_empty() {
        return Vec::new();
    }
    let mut rng = Rng::new(seed);
    let mut queries = Vec::with_capacity(count);
    for i in 0..count {
        let mut ty = QueryType::ALL[i % QueryType::ALL.len()];
        let kind = [Kind::Track, Kind::Track, Kind::Album, Kind::Artist][rng.below(4)];
        let mut picked = target(entries[rng.below(entries.len())], kind);
        if ty == QueryType::Diacritics {
            // Look for a name that folding changes a few times, then fall back to a
            // prefix query so the count stays what was asked.
            let found = (0..200)
                .map(|_| target(entries[rng.below(entries.len())], kind))
                .find(|(n, _)| foldable(n));
            match found {
                Some(t) => picked = t,
                None => ty = QueryType::Prefix,
            }
        }
        let (name, artist) = picked;
        let nchars = name.chars().count();
        let text = match ty {
            QueryType::Prefix => {
                let min = 3.min(nchars);
                let n = min + rng.below(nchars - min + 1);
                name.chars().take(n).collect()
            }
            QueryType::Typo => make_typo(&name, &mut rng),
            QueryType::Diacritics => fold(&name),
            QueryType::Multiword => match artist {
                Some(a) => format!("{a} {name}"),
                None => name,
            },
        };
        queries.push(Query { ty, text });
    }
    queries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(title: &str, artist: &str, album: Option<&str>) -> Entry {
        Entry {
            path: format!("{artist}/{title}.mp3"),
            title: title.into(),
            artist: artist.into(),
            album: album.map(Into::into),
            album_artist: Some(artist.into()),
        }
    }

    fn sample() -> Vec<Entry> {
        vec![
            entry("Étoile du Nord", "Østergade", Some("Ångström")),
            entry("Silent River", "Mira Fontana", Some("Parade of Anchor")),
            entry(
                "The Lantern Remembering",
                "Alma Baker",
                Some("Harbor Songs"),
            ),
            entry("Burning Satellite", "Anna Berg", None),
        ]
    }

    #[test]
    fn fold_strips_diacritics_and_special_letters() {
        assert_eq!(fold("Étoile du Nord"), "etoile du nord");
        assert_eq!(fold("Østergade"), "ostergade");
        assert_eq!(fold("Straße"), "strasse");
        assert_eq!(fold("Łódź"), "lodz");
        assert_eq!(fold("日本語"), "日本語");
    }

    #[test]
    fn cjk_is_detected() {
        assert!(has_cjk("夜の音楽"));
        assert!(has_cjk("Song 노래"));
        assert!(!has_cjk("Étoile du Nord"));
        assert!(!has_cjk(""));
    }

    #[test]
    fn same_inputs_give_the_same_queries() {
        let a = build(&sample(), 200, SEED);
        let b = build(&sample(), 200, SEED);
        assert_eq!(a, b);
        assert_eq!(a.len(), 200);
        let c = build(&sample(), 200, SEED + 1);
        assert_ne!(a, c);
    }

    #[test]
    fn the_types_are_in_equal_rotation() {
        let q = build(&sample(), 400, SEED);
        for ty in QueryType::ALL {
            let n = q.iter().filter(|x| x.ty == ty).count();
            assert_eq!(n, 100, "{ty:?}");
        }
    }

    #[test]
    fn prefixes_are_at_least_three_characters_of_a_name() {
        let names: Vec<String> = sample()
            .into_iter()
            .flat_map(|e| [e.title, e.artist, e.album.unwrap_or_default()])
            .collect();
        let q = build(&sample(), 400, SEED);
        for q in q.iter().filter(|q| q.ty == QueryType::Prefix) {
            assert!(q.text.chars().count() >= 3, "{}", q.text);
            assert!(names.iter().any(|n| n.starts_with(&q.text)), "{}", q.text);
        }
    }

    #[test]
    fn diacritics_queries_are_folded() {
        let q = build(&sample(), 400, SEED);
        let d: Vec<_> = q.iter().filter(|q| q.ty == QueryType::Diacritics).collect();
        assert_eq!(d.len(), 100);
        for q in d {
            assert_eq!(q.text, fold(&q.text));
            assert!(
                ["etoile du nord", "ostergade", "angstrom"].contains(&q.text.as_str()),
                "{}",
                q.text
            );
        }
    }

    #[test]
    fn without_foldable_names_diacritics_become_prefixes() {
        let plain = vec![entry("Silent River", "Mira Fontana", Some("Parade"))];
        let q = build(&plain, 40, SEED);
        assert_eq!(q.len(), 40);
        assert!(q.iter().all(|q| q.ty != QueryType::Diacritics));
    }

    #[test]
    fn a_typo_is_one_edit_away() {
        let mut rng = Rng::new(SEED);
        for name in [
            "Silent River",
            "The Lantern Remembering",
            "Étoile du Nord",
            "ab",
            "x",
        ] {
            for _ in 0..50 {
                let t = make_typo(name, &mut rng);
                let (a, b): (Vec<char>, Vec<char>) = (name.chars().collect(), t.chars().collect());
                assert!(a.len().abs_diff(b.len()) <= 1, "{name} -> {t}");
                if a.len() == b.len() {
                    let diff = a.iter().zip(&b).filter(|(x, y)| x != y).count();
                    assert!(diff <= 1, "{name} -> {t}");
                }
            }
        }
    }

    #[test]
    fn multiword_puts_the_artist_before_the_name() {
        let q = build(&sample(), 400, SEED);
        assert!(
            q.iter()
                .any(|q| q.ty == QueryType::Multiword && q.text == "Mira Fontana Silent River")
        );
    }

    #[test]
    fn an_empty_manifest_gives_no_queries() {
        assert!(build(&[], 10, SEED).is_empty());
    }
}
