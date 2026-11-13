//! Artist credits and artist identity (spec §8.1 "Identità degli artisti" and §8.7
//! point 7).
//!
//! A credit such as `"Queen feat. David Bowie"` is split into the artists it names only
//! when the split is safe ([`split_credit`]), then each artist is found or created
//! ([`resolve_artists`]). An artist is identified by its MusicBrainz id when known,
//! otherwise by its normalized name.

use rusqlite::{OptionalExtension, Transaction, params};

use crate::Result;
use crate::db::now_ms;
use crate::ids::{ArtistId, LibraryId};
use crate::text::normalize;

/// One artist of a credit, like a row of `artist_credit_name` in MusicBrainz.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreditArtist {
    /// The name as written in this credit.
    pub credited_name: String,
    /// The text of the credit that follows this name (`" feat. "`, `"; "`); usually
    /// empty for the last artist.
    pub join_phrase: String,
    /// MusicBrainz artist id, if the tags give one for this artist.
    pub mbid: Option<String>,
}

/// Splits the credit `credit` into its artists, using the multi-value `ARTISTS` tag
/// (`artists_tag`) and the MusicBrainz artist ids (`mbids`) when present. The rules of
/// §8.1, in order:
///
/// 1. If `mbids` is not empty and `artists_tag` has the same length, one artist per
///    name and mbid pair. If `mbids` has a single value, the whole credit is one artist
///    with that mbid. With different lengths and more than one mbid, the mbids are
///    ignored.
/// 2. If `artists_tag` is not empty, one artist per value.
/// 3. Otherwise the credit text is split only on `feat.`, `ft.` and `featuring` (whole
///    words, any case, also inside brackets as in `"A (ft. B)"`) and on `;`. Never on
///    `&`, `and`, `e`, `,`, `/` or `x`: "Simon & Garfunkel" stays one artist. A credit
///    that cannot be split safely (an empty part, a bracket closed before more text)
///    stays one artist.
///
/// With names from the text, concatenating `credited_name` and `join_phrase` of every
/// entry gives back the credit (trimmed); a bracket opened in a join phrase is closed
/// in the following join phrase, so `"A (ft. B)"` gives `"A"`, `" (ft. "`, `"B"`, `")"`.
/// With names from `artists_tag`, the join phrase of a name is the text of the credit
/// between it and the next name when both are found in order in the credit (as whole
/// words, ignoring case), otherwise empty.
///
/// Values of `artists_tag` and `mbids` are trimmed and empty values are dropped. An
/// empty credit with no `ARTISTS` values gives an empty list.
pub fn split_credit(credit: &str, artists_tag: &[String], mbids: &[String]) -> Vec<CreditArtist> {
    let credit = credit.trim();
    let names = non_empty(artists_tag);
    let mbids = non_empty(mbids);
    if !mbids.is_empty() {
        if names.len() == mbids.len() {
            return from_names(credit, &names, Some(&mbids));
        }
        if mbids.len() == 1 && !credit.is_empty() {
            return vec![whole(credit, Some(mbids[0]))];
        }
    }
    if !names.is_empty() {
        return from_names(credit, &names, None);
    }
    if credit.is_empty() {
        return Vec::new();
    }
    split_text(credit).unwrap_or_else(|| vec![whole(credit, None)])
}

fn non_empty(values: &[String]) -> Vec<&str> {
    values
        .iter()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .collect()
}

fn whole(credit: &str, mbid: Option<&str>) -> CreditArtist {
    CreditArtist {
        credited_name: credit.to_owned(),
        join_phrase: String::new(),
        mbid: mbid.map(str::to_owned),
    }
}

/// One artist per name, with join phrases taken from the credit (rules 1 and 2).
fn from_names(credit: &str, names: &[&str], mbids: Option<&[&str]>) -> Vec<CreditArtist> {
    let mut joins = vec![String::new(); names.len()];
    let mut cursor = 0;
    // End in the credit of the previous name, if it was found in order.
    let mut prev_end: Option<usize> = None;
    for (i, name) in names.iter().enumerate() {
        match find_word_ci(credit, name, cursor) {
            Some((start, end)) => {
                if let Some(prev) = prev_end {
                    joins[i - 1] = credit[prev..start].to_owned();
                }
                prev_end = Some(end);
                cursor = end;
            }
            None => prev_end = None,
        }
    }
    names
        .iter()
        .zip(joins)
        .enumerate()
        .map(|(i, (name, join_phrase))| CreditArtist {
            credited_name: (*name).to_owned(),
            join_phrase,
            mbid: mbids.map(|m| m[i].to_owned()),
        })
        .collect()
}

/// Byte range of the first match of `needle` in `hay` at or after byte `from`, ignoring
/// case. A match must not start or end inside a word.
fn find_word_ci(hay: &str, needle: &str, from: usize) -> Option<(usize, usize)> {
    let starts = hay[from..].char_indices().map(|(i, _)| from + i);
    for start in starts {
        if hay[..start]
            .chars()
            .next_back()
            .is_some_and(|before| joins_word(before, needle.chars().next()))
        {
            continue;
        }
        let Some(end) = match_ci(&hay[start..], needle).map(|len| start + len) else {
            continue;
        };
        if hay[end..]
            .chars()
            .next()
            .is_some_and(|after| joins_word(after, needle.chars().next_back()))
        {
            continue;
        }
        return Some((start, end));
    }
    None
}

/// True if `outside` next to the edge character `edge` of a match would continue the
/// same word.
fn joins_word(outside: char, edge: Option<char>) -> bool {
    outside.is_alphanumeric() && edge.is_some_and(char::is_alphanumeric)
}

/// Length in bytes of the prefix of `hay` equal to `needle` ignoring case, if any.
fn match_ci(hay: &str, needle: &str) -> Option<usize> {
    let mut hay_chars = hay.char_indices();
    for n in needle.chars() {
        let (_, h) = hay_chars.next()?;
        if h != n && !h.to_lowercase().eq(n.to_lowercase()) {
            return None;
        }
    }
    Some(hay_chars.next().map_or(hay.len(), |(i, _)| i))
}

/// Words that separate artists in a credit, compared ignoring ASCII case.
const FEAT_WORDS: [&str; 3] = ["featuring", "feat.", "ft."];

/// A separator in the credit text: the byte range of its join phrase, and the closing
/// bracket expected if the join phrase opens one.
struct Separator {
    start: usize,
    end: usize,
    closer: Option<char>,
}

/// Splits the credit text on the safe separators (rule 3). `None` if the credit cannot
/// be split safely.
fn split_text(credit: &str) -> Option<Vec<CreditArtist>> {
    let mut separators = find_separators(credit);
    separators.push(Separator {
        start: credit.len(),
        end: credit.len(),
        closer: None,
    });
    let mut out = Vec::with_capacity(separators.len());
    let mut name_start = 0;
    let mut open: Option<char> = None;
    for sep in separators {
        if sep.start <= name_start {
            return None;
        }
        let mut name = &credit[name_start..sep.start];
        if let Some(closer) = open {
            if let Some(rest) = name.strip_suffix(closer) {
                name = rest.trim_end();
                open = None;
            }
            // A bracket closed before more text, or an empty name inside the brackets.
            if name.contains(closer) || name.is_empty() {
                return None;
            }
        }
        if let Some(closer) = sep.closer {
            if open.is_some() {
                return None;
            }
            open = Some(closer);
        }
        let name_end = name_start + name.len();
        out.push(CreditArtist {
            credited_name: name.to_owned(),
            join_phrase: credit[name_end..sep.end].to_owned(),
            mbid: None,
        });
        name_start = sep.end;
    }
    (out.len() > 1).then_some(out)
}

/// The separators of `credit` (trimmed), each widened to the spaces around it and to an
/// opening bracket just before it.
fn find_separators(credit: &str) -> Vec<Separator> {
    let mut out = Vec::new();
    let mut prev: Option<char> = None;
    let mut skip_to = 0;
    for (i, c) in credit.char_indices() {
        if i < skip_to {
            prev = Some(c);
            continue;
        }
        let len = if c == ';' {
            Some(1)
        } else if prev.is_some_and(|p| p.is_whitespace() || p == '(' || p == '[') {
            feat_word_at(&credit[i..])
        } else {
            None
        };
        prev = Some(c);
        let Some(len) = len else { continue };
        skip_to = i + len;
        out.push(widen(credit, i, i + len));
    }
    out
}

/// Length of the separator word at the start of `rest`, if there is one.
fn feat_word_at(rest: &str) -> Option<usize> {
    FEAT_WORDS.iter().find_map(|word| {
        let head = rest.get(..word.len())?;
        if !head.eq_ignore_ascii_case(word) {
            return None;
        }
        let next = rest[word.len()..].chars().next();
        let ends_word = if word.ends_with('.') {
            next.is_none_or(char::is_whitespace)
        } else {
            next.is_none_or(|n| !n.is_alphanumeric())
        };
        ends_word.then_some(word.len())
    })
}

/// Widens the separator `start..end` over the spaces around it and over an opening
/// bracket (with its spaces) just before it.
fn widen(credit: &str, start: usize, end: usize) -> Separator {
    let mut start = start - trailing_spaces(&credit[..start]);
    let mut closer = None;
    if let Some(open) = credit[..start].chars().next_back() {
        let close = match open {
            '(' => Some(')'),
            '[' => Some(']'),
            _ => None,
        };
        if close.is_some() {
            closer = close;
            start -= open.len_utf8();
            start -= trailing_spaces(&credit[..start]);
        }
    }
    let rest = &credit[end..];
    let end = end + (rest.len() - rest.trim_start().len());
    Separator { start, end, closer }
}

fn trailing_spaces(s: &str) -> usize {
    s.len() - s.trim_end().len()
}

/// An artist found or created by [`resolve_artist_keys`]. Core-internal: `key` is the
/// internal key, for inserts into `track_artists` and `album_artists` inside the core.
pub(crate) struct ResolvedArtist {
    pub(crate) key: i64,
    pub(crate) id: ArtistId,
}

/// Finds or creates the artist of every entry of `list`, in order. The caller then
/// stores in `track_artists` or `album_artists`, by position, the artist with the
/// entry's `credited_name` and `join_phrase`.
///
/// - With an mbid: the artist with that mbid, created if missing; its name follows the
///   credit when it changes.
/// - Without an mbid: if exactly one artist with an mbid and the same normalized name
///   is linked to tracks or albums of `library`, that artist (§8.7 point 7); otherwise
///   the artist without mbid with that normalized name, created if missing. Homonyms
///   (more than one such mbid artist) keep the name-only tracks on a separate artist.
///
/// `sort` (the `ARTISTSORT` or `ALBUMARTISTSORT` tag) is stored as the artist's
/// `sort_name` only when the credit has a single artist. Every change to an artist sets
/// its `updated_at`.
pub fn resolve_artists(
    tx: &Transaction<'_>,
    library: LibraryId,
    list: &[CreditArtist],
    sort: Option<&str>,
) -> Result<Vec<ArtistId>> {
    Ok(resolve_artist_keys(tx, library, list, sort)?
        .into_iter()
        .map(|a| a.id)
        .collect())
}

/// Same as [`resolve_artists`], also returning the internal keys. Core-internal.
pub(crate) fn resolve_artist_keys(
    tx: &Transaction<'_>,
    library: LibraryId,
    list: &[CreditArtist],
    sort: Option<&str>,
) -> Result<Vec<ResolvedArtist>> {
    let now = now_ms();
    let sort = sort
        .map(str::trim)
        .filter(|s| !s.is_empty() && list.len() == 1);
    let mut out = Vec::with_capacity(list.len());
    for artist in list {
        let resolved = match artist.mbid.as_deref() {
            Some(mbid) => by_mbid(tx, mbid, &artist.credited_name, now)?,
            None => by_name(tx, library, &artist.credited_name, now)?,
        };
        if let Some(sort) = sort {
            tx.prepare_cached(
                "UPDATE artists SET sort_name = ?2, updated_at = ?3
                 WHERE id = ?1 AND sort_name IS NOT ?2",
            )?
            .execute(params![resolved.key, sort, now])?;
        }
        out.push(resolved);
    }
    Ok(out)
}

/// Normalized name used for artist identity. A name made only of punctuation (like the
/// band "!!!") normalizes to nothing, so it keeps its own lowercase text instead.
fn norm_name(name: &str) -> String {
    let norm = normalize(name);
    if norm.is_empty() {
        name.trim().to_lowercase()
    } else {
        norm
    }
}

fn by_mbid(tx: &Transaction<'_>, mbid: &str, name: &str, now: i64) -> Result<ResolvedArtist> {
    let found = tx
        .prepare_cached("SELECT id, public_id, name FROM artists WHERE mbid = ?1")?
        .query_row([mbid], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, ArtistId>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .optional()?;
    match found {
        Some((key, id, current)) => {
            if current != name {
                tx.prepare_cached(
                    "UPDATE artists SET name = ?2, norm_name = ?3, updated_at = ?4 WHERE id = ?1",
                )?
                .execute(params![key, name, norm_name(name), now])?;
            }
            Ok(ResolvedArtist { key, id })
        }
        None => insert(tx, name, Some(mbid), now),
    }
}

fn by_name(
    tx: &Transaction<'_>,
    library: LibraryId,
    name: &str,
    now: i64,
) -> Result<ResolvedArtist> {
    let norm = norm_name(name);
    let linked: Vec<(i64, ArtistId)> = tx
        .prepare_cached(
            "SELECT a.id, a.public_id FROM artists a
             WHERE a.norm_name = ?1 AND a.mbid IS NOT NULL
               AND (EXISTS (SELECT 1 FROM track_artists ta JOIN tracks t ON t.id = ta.track_id
                            WHERE ta.artist_id = a.id AND t.library_id = ?2)
                    OR EXISTS (SELECT 1 FROM album_artists aa
                               JOIN library_albums al ON al.id = aa.album_id
                               WHERE aa.artist_id = a.id AND al.library_id = ?2))
             LIMIT 2",
        )?
        .query_map(params![norm, library.0], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    if let [(key, id)] = linked.as_slice() {
        return Ok(ResolvedArtist {
            key: *key,
            id: id.clone(),
        });
    }
    let found = tx
        .prepare_cached("SELECT id, public_id FROM artists WHERE norm_name = ?1 AND mbid IS NULL")?
        .query_row([&norm], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    match found {
        Some((key, id)) => Ok(ResolvedArtist { key, id }),
        None => insert(tx, name, None, now),
    }
}

fn insert(
    tx: &Transaction<'_>,
    name: &str,
    mbid: Option<&str>,
    now: i64,
) -> Result<ResolvedArtist> {
    let id = ArtistId::generate();
    tx.prepare_cached(
        "INSERT INTO artists (public_id, name, norm_name, mbid, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?
    .execute(params![id, name, norm_name(name), mbid, now])?;
    Ok(ResolvedArtist {
        key: tx.last_insert_rowid(),
        id,
    })
}
