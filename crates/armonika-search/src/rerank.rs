//! Engine-agnostic final ranking over the candidates of the engine.
//!
//! A faithful port of `score` and `rerank_group` of the search spike
//! (`spikes/01-search/src/common.rs`), plus the popularity signal: the final cost of a
//! candidate is `cost - boost`. Ties keep the order of the engine.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// The text of a candidate: tracks hold (title, credit, album), albums (title, credit,
/// empty), artists (name, empty, empty).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Doc {
    pub a: String,
    pub b: String,
    pub c: String,
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

/// Folded tokens.
pub fn tokens(s: &str) -> Vec<String> {
    fold(s)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect()
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

/// Cost of `doc` for the query tokens `qtoks`: the edit distance of each query word to
/// its closest word of the doc (the last query word may be a prefix), with 0.3 added
/// when the closest word is not in the title, and a small penalty for long docs.
fn score(qtoks: &[Vec<char>], doc: &Doc) -> f32 {
    let fields: [&str; 3] = [&doc.a, &doc.b, &doc.c];
    let dtoks: Vec<Vec<Vec<char>>> = fields
        .iter()
        .map(|f| tokens(f).iter().map(|t| t.chars().collect()).collect())
        .collect();
    let total_doc_tokens: usize = dtoks.iter().map(Vec::len).sum();
    let mut cost = 0.0f32;
    for (i, qt) in qtoks.iter().enumerate() {
        let is_last = i + 1 == qtoks.len();
        let mut best = f32::MAX;
        for (fi, ft) in dtoks.iter().enumerate() {
            let w = if fi > 0 { 0.3 } else { 0.0 };
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

/// The indexes of `docs` best first. `boost(i)` is subtracted from the cost of
/// `docs[i]`. A query without words (only CJK text, which the engine matched on its
/// own) gets the same cost for every doc, so the order of the engine decides, after
/// the boost.
pub fn rerank_group(query: &str, docs: &[Doc], boost: &dyn Fn(usize) -> f32) -> Vec<usize> {
    let qtoks: Vec<Vec<char>> = tokens(query).iter().map(|t| t.chars().collect()).collect();
    let mut scored: Vec<(f32, usize)> = docs
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let cost = if qtoks.is_empty() {
                0.0
            } else {
                score(&qtoks, d)
            };
            (cost - boost(i), i)
        })
        .collect();
    scored.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
    scored.into_iter().map(|(_, i)| i).collect()
}
