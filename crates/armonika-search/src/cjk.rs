//! Bigram tokenizer for CJK text (Han, Hiragana, Katakana, Hangul).
//!
//! These scripts are written without spaces, so words cannot be told apart. Each run of
//! CJK characters yields its pairs of consecutive characters, or the character itself
//! for a run of one. A query is cut the same way, so any substring of two or more
//! characters is found.

use tantivy::tokenizer::{Token, TokenStream, Tokenizer};

/// Name under which the tokenizer is registered in Tantivy.
pub const TOKENIZER_NAME: &str = "cjk_bigram";

/// Whether `c` is a Han, Hiragana, Katakana or Hangul character.
pub fn is_cjk(c: char) -> bool {
    matches!(u32::from(c),
        0x1100..=0x11FF       // Hangul Jamo
        | 0x2E80..=0x2FDF     // CJK radicals
        | 0x3040..=0x309F     // Hiragana
        | 0x30A0..=0x30FF     // Katakana
        | 0x3130..=0x318F     // Hangul compatibility Jamo
        | 0x31F0..=0x31FF     // Katakana phonetic extensions
        | 0x3400..=0x4DBF     // Han extension A
        | 0x4E00..=0x9FFF     // Han
        | 0xA960..=0xA97F     // Hangul Jamo extended A
        | 0xAC00..=0xD7FF     // Hangul syllables and Jamo extended B
        | 0xF900..=0xFAFF     // Han compatibility
        | 0xFF66..=0xFF9F     // Halfwidth Katakana
        | 0x20000..=0x323AF   // Han extensions B and later
    )
}

/// The runs of CJK characters of `text`, in order.
pub fn sequences(text: &str) -> Vec<String> {
    runs(text).into_iter().map(|(_, run)| run).collect()
}

/// `text` with every CJK character replaced by a space.
pub fn without_cjk(text: &str) -> String {
    text.chars()
        .map(|c| if is_cjk(c) { ' ' } else { c })
        .collect()
}

/// The bigrams of the run `seq`, or the run itself when it is one character long.
pub fn ngrams(seq: &str) -> Vec<String> {
    let chars: Vec<char> = seq.chars().collect();
    match chars.len() {
        0 => Vec::new(),
        1 => vec![seq.to_owned()],
        _ => chars.windows(2).map(|w| w.iter().collect()).collect(),
    }
}

/// Runs with the byte offset where each starts.
fn runs(text: &str) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::new();
    let mut open = false;
    for (at, c) in text.char_indices() {
        if is_cjk(c) {
            match out.last_mut() {
                Some((_, run)) if open => run.push(c),
                _ => out.push((at, c.to_string())),
            }
            open = true;
        } else {
            open = false;
        }
    }
    out
}

/// The `cjk_bigram` tokenizer.
#[derive(Clone, Copy, Debug, Default)]
pub struct CjkBigramTokenizer;

/// Token stream over the bigrams found, computed up front.
pub struct CjkBigramStream {
    tokens: Vec<Token>,
    next: usize,
}

impl Tokenizer for CjkBigramTokenizer {
    type TokenStream<'a> = CjkBigramStream;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> CjkBigramStream {
        let mut tokens = Vec::new();
        for (start, run) in runs(text) {
            let chars: Vec<(usize, char)> = run.char_indices().collect();
            let end_of = |i: usize| start + chars.get(i + 1).map_or(run.len(), |(at, _)| *at);
            if chars.len() == 1 {
                tokens.push(Token {
                    offset_from: start,
                    offset_to: end_of(0),
                    position: tokens.len(),
                    text: run.clone(),
                    position_length: 1,
                });
                continue;
            }
            for i in 0..chars.len() - 1 {
                tokens.push(Token {
                    offset_from: start + chars[i].0,
                    offset_to: end_of(i + 1),
                    position: tokens.len(),
                    text: run[chars[i].0..chars[i + 1].0 + chars[i + 1].1.len_utf8()].to_owned(),
                    position_length: 2,
                });
            }
        }
        CjkBigramStream { tokens, next: 0 }
    }
}

impl TokenStream for CjkBigramStream {
    fn advance(&mut self) -> bool {
        if self.next < self.tokens.len() {
            self.next += 1;
            true
        } else {
            false
        }
    }

    fn token(&self) -> &Token {
        &self.tokens[self.next - 1]
    }

    fn token_mut(&mut self) -> &mut Token {
        &mut self.tokens[self.next - 1]
    }
}
