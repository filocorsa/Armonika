//! Artist credits and artist identity (spec §8.1 "Identità degli artisti" and §8.7
//! point 7).
//!
//! A credit such as `"Queen feat. David Bowie"` is split into the artists it names only
//! when the split is safe ([`split_credit`]), then each artist is found or created
//! ([`resolve_artists`]) and linked to its track or album ([`link_track_artists`],
//! [`link_album_artists`]). An artist is identified by its MusicBrainz id when known,
//! otherwise by its normalized name ([`norm_name`]). [`reconcile_artists`] re-applies
//! the rule for name-only credits at the end of a scan, so the result does not depend
//! on scan order.

use std::collections::{BTreeSet, HashMap};

use rusqlite::{OptionalExtension, Transaction, params};

use crate::db::now_ms;
use crate::ids::{ArtistId, LibraryAlbumId, LibraryId, TrackId, resolve_album, resolve_track};
use crate::text::normalize;
use crate::{Error, Result};

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
    let last = separators.len() - 1;
    let mut out = Vec::with_capacity(separators.len());
    let mut name_start = 0;
    let mut open: Option<char> = None;
    for (i, sep) in separators.into_iter().enumerate() {
        if sep.start <= name_start {
            return None;
        }
        let mut name = &credit[name_start..sep.start];
        if let Some(closer) = open {
            if let Some(rest) = name.strip_suffix(closer) {
                name = rest.trim_end();
                open = None;
            }
            // A bracket closed before more text, an empty name inside the brackets, or
            // another separator inside the brackets ("A (ft. B; C)").
            if name.contains(closer) || name.is_empty() || (open.is_some() && i != last) {
                return None;
            }
        }
        if sep.closer.is_some() {
            open = sep.closer;
        }
        let name_end = name_start + name.len();
        out.push(CreditArtist {
            credited_name: name.to_owned(),
            join_phrase: credit[name_end..sep.end].to_owned(),
            mbid: None,
        });
        name_start = sep.end;
    }
    // An unclosed bracket ("A (ft. B") is not a safe split either.
    (out.len() > 1 && open.is_none()).then_some(out)
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

/// Normalized name of an artist: the only normalization used to match artists by name
/// (`artists.norm_name`, the unique-name rule, [`reconcile_artists`]). It is
/// [`normalize`], except that a name made only of punctuation (like the band "!!!"),
/// which normalizes to nothing, keeps its own lowercase text.
pub(crate) fn norm_name(name: &str) -> String {
    let norm = normalize(name);
    if norm.is_empty() {
        name.trim().to_lowercase()
    } else {
        norm
    }
}

/// An artist found or created for one entry of a credit.
struct Resolved {
    key: i64,
    id: ArtistId,
    /// The entry had no mbid and was attached to an artist with one by the unique-name
    /// rule.
    matched_by_name: bool,
}

/// Condition on the artist `a`: linked to tracks or albums of the library `?1` by rows
/// that came from a real mbid (not themselves matched by name).
const LINKED_IN_LIBRARY: &str = "(EXISTS (SELECT 1 FROM track_artists ta
                     JOIN tracks t ON t.id = ta.track_id
                     WHERE ta.artist_id = a.id AND ta.matched_by_name = 0
                       AND t.library_id = ?1)
         OR EXISTS (SELECT 1 FROM album_artists aa
                    JOIN library_albums al ON al.id = aa.album_id
                    WHERE aa.artist_id = a.id AND aa.matched_by_name = 0
                      AND al.library_id = ?1))";

/// Finds or creates the artist of every entry of `list`, in order. Prefer
/// [`link_track_artists`] and [`link_album_artists`], which also store the links.
///
/// - With an mbid: the artist with that mbid, created if missing; its name follows the
///   credit when it changes.
/// - Without an mbid: if exactly one artist with an mbid and the same normalized name
///   is linked (through a credit that had the mbid) to tracks or albums of `library`,
///   that artist (§8.7 point 7); otherwise the artist without mbid with that normalized
///   name, created if missing. Homonyms (more than one such mbid artist) keep the
///   name-only credits on a separate artist. The result depends on what is already
///   linked; [`reconcile_artists`] makes it independent of scan order.
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
    Ok(resolve_list(tx, library, list, sort)?
        .into_iter()
        .map(|a| a.id)
        .collect())
}

fn resolve_list(
    tx: &Transaction<'_>,
    library: LibraryId,
    list: &[CreditArtist],
    sort: Option<&str>,
) -> Result<Vec<Resolved>> {
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

fn by_mbid(tx: &Transaction<'_>, mbid: &str, name: &str, now: i64) -> Result<Resolved> {
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
    let (key, id) = match found {
        Some((key, id, current)) => {
            if current != name {
                tx.prepare_cached(
                    "UPDATE artists SET name = ?2, norm_name = ?3, updated_at = ?4 WHERE id = ?1",
                )?
                .execute(params![key, name, norm_name(name), now])?;
            }
            (key, id)
        }
        None => insert(tx, name, Some(mbid), now)?,
    };
    Ok(Resolved {
        key,
        id,
        matched_by_name: false,
    })
}

fn by_name(tx: &Transaction<'_>, library: LibraryId, name: &str, now: i64) -> Result<Resolved> {
    let norm = norm_name(name);
    let sql = format!(
        "SELECT a.id, a.public_id FROM artists a
         WHERE a.norm_name = ?2 AND a.mbid IS NOT NULL AND {LINKED_IN_LIBRARY}
         LIMIT 2"
    );
    let linked: Vec<(i64, ArtistId)> = tx
        .prepare_cached(&sql)?
        .query_map(params![library.0, norm], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    if let [(key, id)] = linked.as_slice() {
        return Ok(Resolved {
            key: *key,
            id: id.clone(),
            matched_by_name: true,
        });
    }
    let (key, id) = name_only(tx, &norm, name, now)?;
    Ok(Resolved {
        key,
        id,
        matched_by_name: false,
    })
}

/// The artist without mbid for the normalized name `norm`, created with the display
/// name `name` if missing.
fn name_only(tx: &Transaction<'_>, norm: &str, name: &str, now: i64) -> Result<(i64, ArtistId)> {
    let found = tx
        .prepare_cached("SELECT id, public_id FROM artists WHERE norm_name = ?1 AND mbid IS NULL")?
        .query_row([norm], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    match found {
        Some(found) => Ok(found),
        None => insert(tx, name, None, now),
    }
}

fn insert(
    tx: &Transaction<'_>,
    name: &str,
    mbid: Option<&str>,
    now: i64,
) -> Result<(i64, ArtistId)> {
    let id = ArtistId::generate();
    tx.prepare_cached(
        "INSERT INTO artists (public_id, name, norm_name, mbid, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?
    .execute(params![id, name, norm_name(name), mbid, now])?;
    Ok((tx.last_insert_rowid(), id))
}

/// A table of artist links (`track_artists` or `album_artists`) and its owner table.
struct LinkTable {
    links: &'static str,
    owner_col: &'static str,
    owners: &'static str,
}

const TRACK_LINKS: LinkTable = LinkTable {
    links: "track_artists",
    owner_col: "track_id",
    owners: "tracks",
};

const ALBUM_LINKS: LinkTable = LinkTable {
    links: "album_artists",
    owner_col: "album_id",
    owners: "library_albums",
};

/// Resolves the artists of the credit `list` ([`resolve_artists`], in the track's
/// library) and replaces the track's rows of `track_artists` with them, by position,
/// with each entry's `credited_name` and `join_phrase`. Returns the artists in order.
pub fn link_track_artists(
    tx: &Transaction<'_>,
    track: &TrackId,
    list: &[CreditArtist],
    sort: Option<&str>,
) -> Result<Vec<ArtistId>> {
    let key = resolve_track(tx, track)?.ok_or_else(|| Error::TrackNotFound(track.clone()))?;
    link(tx, &TRACK_LINKS, key, list, sort)
}

/// Same as [`link_track_artists`] for the artists of a library album (`album_artists`).
pub fn link_album_artists(
    tx: &Transaction<'_>,
    album: &LibraryAlbumId,
    list: &[CreditArtist],
    sort: Option<&str>,
) -> Result<Vec<ArtistId>> {
    let key = resolve_album(tx, album)?.ok_or_else(|| Error::AlbumNotFound(album.clone()))?;
    link(tx, &ALBUM_LINKS, key, list, sort)
}

fn link(
    tx: &Transaction<'_>,
    table: &LinkTable,
    owner: i64,
    list: &[CreditArtist],
    sort: Option<&str>,
) -> Result<Vec<ArtistId>> {
    let LinkTable {
        links,
        owner_col,
        owners,
    } = table;
    let library: i64 = tx
        .prepare_cached(&format!("SELECT library_id FROM {owners} WHERE id = ?1"))?
        .query_row([owner], |r| r.get(0))?;
    // The old rows go first, so they do not count as links of the library.
    tx.prepare_cached(&format!("DELETE FROM {links} WHERE {owner_col} = ?1"))?
        .execute([owner])?;
    let resolved = resolve_list(tx, LibraryId(library), list, sort)?;
    let mut insert = tx.prepare_cached(&format!(
        "INSERT INTO {links} ({owner_col}, position, artist_id, credited_name, join_phrase,
                              matched_by_name)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
    ))?;
    for (position, (entry, artist)) in (0_i64..).zip(list.iter().zip(&resolved)) {
        insert.execute(params![
            owner,
            position,
            artist.key,
            entry.credited_name,
            entry.join_phrase,
            artist.matched_by_name
        ])?;
    }
    Ok(resolved.into_iter().map(|a| a.id).collect())
}

/// Re-applies the identity rule of §8.7 point 7 to every artist link of `library`
/// whose credit had no mbid (links to name-only artists and links matched by name), so
/// that the result does not depend on the order in which files were scanned:
///
/// - if exactly one artist with an mbid and the same normalized name is linked to
///   tracks or albums of the library through a credit with that mbid, the link points
///   to it (matched by name);
/// - otherwise it points to the name-only artist with that normalized name, created if
///   missing.
///
/// Returns the artists whose links changed (both the old and the new artist of each
/// moved link), after setting their `updated_at`. Running it again changes nothing.
/// The scanner calls it once at the end of each scan.
pub fn reconcile_artists(tx: &Transaction<'_>, library: LibraryId) -> Result<Vec<ArtistId>> {
    let now = now_ms();
    // Normalized name to the only mbid artist with it in the library (None if several).
    let mut unique: HashMap<String, Option<i64>> = HashMap::new();
    let sql = format!(
        "SELECT a.id, a.norm_name FROM artists a
         WHERE a.mbid IS NOT NULL AND {LINKED_IN_LIBRARY}"
    );
    let mut stmt = tx.prepare_cached(&sql)?;
    let mut rows = stmt.query([library.0])?;
    while let Some(row) = rows.next()? {
        let (key, norm): (i64, String) = (row.get(0)?, row.get(1)?);
        unique
            .entry(norm)
            .and_modify(|v| *v = None)
            .or_insert(Some(key));
    }
    drop(rows);
    drop(stmt);

    let mut name_only_keys: HashMap<String, i64> = HashMap::new();
    let mut touched: BTreeSet<i64> = BTreeSet::new();
    for table in [&TRACK_LINKS, &ALBUM_LINKS] {
        let LinkTable {
            links,
            owner_col,
            owners,
        } = table;
        let candidates: Vec<(i64, i64, i64, String, bool)> = tx
            .prepare_cached(&format!(
                "SELECT l.{owner_col}, l.position, l.artist_id, l.credited_name,
                        l.matched_by_name
                 FROM {links} l
                 JOIN {owners} o ON o.id = l.{owner_col}
                 JOIN artists a ON a.id = l.artist_id
                 WHERE o.library_id = ?1 AND (a.mbid IS NULL OR l.matched_by_name = 1)"
            ))?
            .query_map([library.0], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut update = tx.prepare_cached(&format!(
            "UPDATE {links} SET artist_id = ?3, matched_by_name = ?4
             WHERE {owner_col} = ?1 AND position = ?2"
        ))?;
        for (owner, position, current, credited, matched) in candidates {
            let norm = norm_name(&credited);
            let target = match unique.get(&norm) {
                Some(Some(key)) => (*key, true),
                _ => {
                    let key = match name_only_keys.get(&norm) {
                        Some(key) => *key,
                        None => {
                            let (key, _) = name_only(tx, &norm, &credited, now)?;
                            name_only_keys.insert(norm, key);
                            key
                        }
                    };
                    (key, false)
                }
            };
            if target != (current, matched) {
                update.execute(params![owner, position, target.0, target.1])?;
                touched.insert(current);
                touched.insert(target.0);
            }
        }
    }

    let mut bump =
        tx.prepare_cached("UPDATE artists SET updated_at = ?2 WHERE id = ?1 RETURNING public_id")?;
    touched
        .into_iter()
        .map(|key| Ok(bump.query_row(params![key, now], |r| r.get(0))?))
        .collect()
}
