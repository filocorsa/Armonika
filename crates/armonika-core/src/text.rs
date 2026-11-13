//! Text folding for matching and sorting.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Folds `s` for matching: NFKD, diacritics removed, lowercase, every non-alphanumeric
/// character turned into a space, spaces collapsed and trimmed.
///
/// Same normalization as spike 4, so "Beyoncé Knowles!" and "beyonce knowles" match.
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for c in s.nfkd().filter(|c| !is_combining_mark(*c)) {
        if c.is_alphanumeric() {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.extend(c.to_lowercase());
        } else {
            pending_space = true;
        }
    }
    out
}

/// Sort key for `s`: [`normalize`]d, without a leading article from `articles`
/// (for example "The" or "L'"). A string made only of an article keeps it.
pub fn fold_sort(s: &str, articles: &[&str]) -> String {
    let folded = normalize(s);
    for article in articles {
        let article = normalize(article);
        if article.is_empty() {
            continue;
        }
        if let Some(rest) = folded
            .strip_prefix(article.as_str())
            .and_then(|rest| rest.strip_prefix(' '))
        {
            return rest.to_owned();
        }
    }
    folded
}
