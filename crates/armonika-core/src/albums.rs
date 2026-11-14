//! Library albums: how tracks are grouped into albums, and how an album keeps its
//! identity (and so its stars) when its tracks are retagged outside Armonika (spec §8.1
//! "Identità degli album della libreria", §8.7 point 10).
//!
//! A track belongs to the album of its group key ([`group_key`]): the MusicBrainz
//! release if known, otherwise the album title and album credit ([`album_credit`]),
//! otherwise the `[Singles]` album of its main artist. Two rules look at the other
//! tracks of the same album folder ([`album_folder`]): a folder whose tracks share an
//! album title but not an artist, without album artist tag, is a compilation credited
//! to "Various Artists" ([`folder_has_mixed_artists`]); a track without release id
//! joins the release of the tracks of its folder with the same title and credit, if
//! they name exactly one release (the folder rule, in [`assign_album`]).
//!
//! The scanner calls [`assign_album`] for every track it writes, so albums appear while
//! the scan runs, and [`reconcile_albums`] once at the end. During the scan a track
//! that already has an album stays in it, even if its key changed: moving it then would
//! lose which album it came from, which is what decides who keeps an identifier when an
//! album splits or merges. [`reconcile_albums`] then recomputes the key of every present
//! track of the library from the database, folder rules included, so the final grouping
//! does not depend on the order of the files or on where batches end; it splits and
//! merges albums keeping identifiers on the larger parts, and recomputes the fields of
//! each album from its tracks.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::artists::{CreditArtist, link_album_artists, norm_name, split_credit};
use crate::db::now_ms;
use crate::ids::{
    ArtistId, LibraryAlbumId, LibraryId, TrackId, add_redirect, resolve_album, resolve_track,
};
use crate::model::{PartialDate, TagFacts};
use crate::text::normalize;
use crate::track_files::split_values;
use crate::{Error, Result};

/// Album credit of compilations without an album artist tag.
pub const VARIOUS_ARTISTS: &str = "Various Artists";

/// Title of the album of the tracks of an artist that have no album title.
pub const SINGLES_TITLE: &str = "[Singles]";

/// The key that groups tracks into a library album, stored in `library_albums.group_key`:
/// `mb:<release mbid>`, `tag:<normalized album title>|<normalized album credit>` or
/// `singles:<artist id>`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GroupKey(String);

impl GroupKey {
    /// The key as stored.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this is the key of the `[Singles]` album of an artist.
    pub fn is_singles(&self) -> bool {
        self.0.starts_with("singles:")
    }

    /// The MusicBrainz release of a `mb:` key.
    fn release(&self) -> Option<&str> {
        self.0.strip_prefix("mb:")
    }

    fn is_tags(&self) -> bool {
        self.0.starts_with("tag:")
    }
}

impl fmt::Display for GroupKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The album credit of a track: its album artist tag; without it, "Various Artists" if
/// the track is marked as part of a compilation or if `folder_has_mixed_artists`
/// (plan decision 2, see [`folder_has_mixed_artists`]); otherwise the credit of the
/// track (empty if it has none).
pub fn album_credit(track: &TagFacts, folder_has_mixed_artists: bool) -> String {
    if let Some(credit) = &track.album_artist_credit {
        return credit.clone();
    }
    if track.compilation || folder_has_mixed_artists {
        return VARIOUS_ARTISTS.to_owned();
    }
    track.artist_credit.clone().unwrap_or_default()
}

/// The group key of a track with tags `tags` and album credit `album_credit`:
/// `mb:<release mbid>` if the tags name the release; otherwise
/// `tag:<normalized album title>|<normalized album credit>`; without album title,
/// `singles:<first_artist>`, the `[Singles]` album of the first artist of the track's
/// credit (§8.7 point 10), one per artist and library. `first_artist` is the artist the
/// track's credit was linked to (`singles:` alone for a track with no artist).
pub fn group_key(tags: &TagFacts, album_credit: &str, first_artist: Option<&ArtistId>) -> GroupKey {
    if let Some(release) = &tags.mb_release_id {
        return GroupKey(format!("mb:{release}"));
    }
    match &tags.album_title {
        Some(title) => tag_key(title, album_credit),
        None => GroupKey(format!(
            "singles:{}",
            first_artist.map_or("", ArtistId::as_str)
        )),
    }
}

fn tag_key(title: &str, album_credit: &str) -> GroupKey {
    GroupKey(format!(
        "tag:{}|{}",
        normalize(title),
        normalize(album_credit)
    ))
}

/// The folder of the album of the track at `rel_path` (database form, `/` separators):
/// the folder of the file, or its parent when the folder is a disc folder (`Disc 1`,
/// `CD2`, `Disk 3 - Live`), so the discs of an album share a folder. `""` for the root.
pub fn album_folder(rel_path: &str) -> &str {
    let parent = rel_path.rfind('/').map_or("", |i| &rel_path[..i]);
    if parent.is_empty() {
        return parent;
    }
    let (grand, name) = match parent.rfind('/') {
        Some(i) => (&parent[..i], &parent[i + 1..]),
        None => ("", parent),
    };
    if is_disc_folder(name) { grand } else { parent }
}

/// `disc`, `disk` or `cd` (any case), optional separators, a number, then nothing or a
/// character that is not a letter or digit.
fn is_disc_folder(name: &str) -> bool {
    let lower = name.trim().to_lowercase();
    ["disc", "disk", "cd"].iter().any(|prefix| {
        let Some(rest) = lower.strip_prefix(prefix) else {
            return false;
        };
        let rest = rest.trim_start_matches([' ', '_', '-', '.']);
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        digits > 0
            && rest[digits..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_alphanumeric())
    })
}

/// A library album, as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryAlbum {
    pub id: LibraryAlbumId,
    pub library: LibraryId,
    pub title: String,
    pub sort_title: Option<String>,
    pub artist_credit: String,
    pub release_mbid: Option<String>,
    pub release_group_mbid: Option<String>,
    pub is_singles: bool,
    /// The most frequent release date among its present tracks.
    pub date: Option<PartialDate>,
    /// The most frequent first release date among its present tracks.
    pub original_date: Option<PartialDate>,
    /// Whether it has at least one present track. Albums whose tracks are all missing
    /// stay, hidden, with their personal data.
    pub visible: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// The album `id`, following a redirect if it was merged into another one.
pub fn get(conn: &Connection, id: &LibraryAlbumId) -> Result<Option<LibraryAlbum>> {
    let Some(key) = resolve_album(conn, id)? else {
        return Ok(None);
    };
    Ok(conn
        .prepare_cached(
            "SELECT a.public_id, a.library_id, a.title, a.sort_title, a.artist_credit,
                    a.release_mbid, a.release_group_mbid, a.is_singles,
                    a.date_y, a.date_m, a.date_d, a.orig_date_y, a.orig_date_m, a.orig_date_d,
                    EXISTS (SELECT 1 FROM tracks t
                            WHERE t.library_album_id = a.id AND t.state = 'present'),
                    a.created_at, a.updated_at
             FROM library_albums a WHERE a.id = ?1",
        )?
        .query_row([key], |r| {
            Ok(LibraryAlbum {
                id: r.get(0)?,
                library: LibraryId(r.get(1)?),
                title: r.get(2)?,
                sort_title: r.get(3)?,
                artist_credit: r.get(4)?,
                release_mbid: r.get(5)?,
                release_group_mbid: r.get(6)?,
                is_singles: r.get(7)?,
                date: date_from(r.get(8)?, r.get(9)?, r.get(10)?),
                original_date: date_from(r.get(11)?, r.get(12)?, r.get(13)?),
                visible: r.get(14)?,
                created_at: r.get(15)?,
                updated_at: r.get(16)?,
            })
        })
        .optional()?)
}

/// The album of the track `track`, if it has one.
pub fn track_album(conn: &Connection, track: &TrackId) -> Result<Option<LibraryAlbumId>> {
    let key = resolve_track(conn, track)?.ok_or_else(|| Error::TrackNotFound(track.clone()))?;
    Ok(conn
        .prepare_cached(
            "SELECT a.public_id FROM tracks t JOIN library_albums a ON a.id = t.library_album_id
             WHERE t.id = ?1",
        )?
        .query_row([key], |r| r.get(0))
        .optional()?)
}

/// The artists of the album `album`, in credit order. Empty for an unknown album.
pub fn album_artist_ids(conn: &Connection, album: &LibraryAlbumId) -> Result<Vec<ArtistId>> {
    let Some(key) = resolve_album(conn, album)? else {
        return Ok(Vec::new());
    };
    Ok(conn
        .prepare_cached(
            "SELECT a.public_id FROM album_artists l JOIN artists a ON a.id = l.artist_id
             WHERE l.album_id = ?1 ORDER BY l.position",
        )?
        .query_map([key], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

/// Every artist linked to an album of `library`, hidden albums included. The scanner
/// compares it before and after [`reconcile_albums`] to learn which artists left an
/// album credit.
pub fn linked_artist_ids(conn: &Connection, library: LibraryId) -> Result<BTreeSet<ArtistId>> {
    Ok(conn
        .prepare_cached(
            "SELECT DISTINCT a.public_id FROM album_artists l
             JOIN library_albums al ON al.id = l.album_id
             JOIN artists a ON a.id = l.artist_id
             WHERE al.library_id = ?1",
        )?
        .query_map([library.0], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

/// Whether the album folder of the track `track` holds other present tracks of
/// `library` with the same album title (normalized) and a different artist credit,
/// counting only tracks without album artist tag (plan decision 2). Read from the
/// database, so it counts the tracks written so far.
pub fn folder_has_mixed_artists(
    conn: &Connection,
    library: LibraryId,
    track: &TrackId,
) -> Result<bool> {
    let key = resolve_track(conn, track)?.ok_or_else(|| Error::TrackNotFound(track.clone()))?;
    let Some(row) = load_rows(conn, library, Filter::Track(key))?.pop() else {
        return Ok(false);
    };
    let Some(title) = row.title_norm.clone() else {
        return Ok(false);
    };
    let rows = load_rows(conn, library, Filter::Title(&title))?;
    Ok(mixed_groups(&rows).contains(&(row.folder.as_str(), title.as_str())))
}

/// Puts the track `track` of `library` (already written, with tags `tags`) in its album
/// and returns the album.
///
/// The folder rule (§8.7 point 10) comes first: a track without release id whose album
/// folder holds tracks with a release id and the same album title and credit (the same
/// `tag:` key) joins their release if they name only one. Then a track without album
/// takes the album with its key (preferring one with present tracks, then the oldest),
/// created if there is none. A new album, or one that had no present track, takes its
/// title, credit, artists and dates from this track; otherwise the fields of the album
/// are left to [`reconcile_albums`], which computes them from all its tracks.
///
/// A track that already has an album stays in it, even if `key` changed: at the end of
/// the scan [`reconcile_albums`] decides splits and merges from the old grouping.
pub fn assign_album(
    tx: &Transaction<'_>,
    library: LibraryId,
    track: &TrackId,
    key: &GroupKey,
    tags: &TagFacts,
) -> Result<LibraryAlbumId> {
    let track_key = resolve_track(tx, track)?.ok_or_else(|| Error::TrackNotFound(track.clone()))?;
    let current: Option<i64> = tx
        .prepare_cached("SELECT library_album_id FROM tracks WHERE id = ?1")?
        .query_row([track_key], |r| r.get(0))?;
    if let Some(album) = current {
        return Ok(tx
            .prepare_cached("SELECT public_id FROM library_albums WHERE id = ?1")?
            .query_row([album], |r| r.get(0))?);
    }

    let row = load_rows(tx, library, Filter::Track(track_key))?
        .pop()
        .ok_or_else(|| Error::TrackNotFound(track.clone()))?;
    // The credit as the folder sees it, and the folder rule.
    let (credit, key) = match &row.title_norm {
        Some(title) => {
            let rows = load_rows(tx, library, Filter::Title(title))?;
            let credits = album_credits(&rows);
            let credit = rows
                .iter()
                .zip(&credits)
                .find(|(r, _)| r.key == track_key)
                .map_or_else(|| album_credit(tags, false), |(_, c)| c.clone());
            let key = if tags.mb_release_id.is_none() && key.is_tags() {
                let releases: BTreeSet<&str> = rows
                    .iter()
                    .zip(&credits)
                    .filter(|(r, _)| r.folder == row.folder)
                    .filter_map(|(r, c)| {
                        let release = r.tags.mb_release_id.as_deref()?;
                        let title = r.tags.album_title.as_deref()?;
                        (tag_key(title, c) == *key).then_some(release)
                    })
                    .collect();
                match releases.into_iter().collect::<Vec<_>>().as_slice() {
                    [release] => GroupKey(format!("mb:{release}")),
                    _ => key.clone(),
                }
            } else {
                key.clone()
            };
            (credit, key)
        }
        None => (album_credit(tags, false), key.clone()),
    };

    let now = now_ms();
    let found: Option<(i64, LibraryAlbumId, bool)> = tx
        .prepare_cached(
            "SELECT a.id, a.public_id,
                    EXISTS (SELECT 1 FROM tracks t
                            WHERE t.library_album_id = a.id AND t.state = 'present')
                      AS visible
             FROM library_albums a WHERE a.library_id = ?1 AND a.group_key = ?2
             ORDER BY visible DESC, a.created_at, a.id LIMIT 1",
        )?
        .query_row(params![library.0, key.as_str()], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?;
    let (album, id, fill) = match found {
        Some((album, id, visible)) => (album, id, !visible),
        None => {
            let (album, id) = insert_album(tx, library, &key, now)?;
            (album, id, true)
        }
    };
    tx.prepare_cached("UPDATE tracks SET library_album_id = ?2 WHERE id = ?1")?
        .execute(params![track_key, album])?;
    tx.prepare_cached("UPDATE library_albums SET updated_at = ?2 WHERE id = ?1")?
        .execute(params![album, now])?;
    if fill {
        let rows = [row];
        let data = album_data(&key, &rows, &[0], std::slice::from_ref(&credit));
        write_data(tx, album, &data, now)?;
        let (list, sort) = credit_list(&key, &rows[0], &credit);
        link_album_artists(tx, &id, &list, sort)?;
    }
    Ok(id)
}

/// Brings the albums of `library` in line with the tags of their tracks, keeping
/// identifiers stable (§8.1); the scanner calls it once at the end of each scan, after
/// `reconcile_artists`. Only present tracks count; albums without present tracks are
/// left alone, hidden, with their personal data.
///
/// 1. The key of every present track is computed again from the database (folder rules
///    included). An album whose tracks now have different keys splits: the part with
///    the most tracks keeps the album (ties: the smaller key) and the album takes its
///    key; the other parts, and present tracks without album, go to the album with
///    their key (the one kept by step 2, else a hidden album with that key, else a new
///    one).
/// 2. Albums that now share a key merge into the one with the most tracks (ties: the
///    oldest). The tracks of the others (missing ones too) move to it, their
///    `album_annotations` are merged into its own (starred if starred in either, with
///    the oldest date; the highest rating), each absorbed identifier redirects to the
///    kept one ([`add_redirect`]) and the absorbed album is deleted.
/// 3. Title, credit, artists, release ids, sort title and dates of every album with
///    present tracks are computed from them (most frequent value; for dates, ties go to
///    the oldest). Every changed album gets a new `updated_at`.
///
/// Returns the albums touched (created, changed, gaining or losing tracks, and the
/// absorbed ones, whose identifiers now redirect). Running it again changes nothing.
pub fn reconcile_albums(tx: &Transaction<'_>, library: LibraryId) -> Result<Vec<LibraryAlbumId>> {
    let now = now_ms();
    let rows = load_rows(tx, library, Filter::All)?;
    let credits = album_credits(&rows);
    let keys = group_keys(&rows, &credits);
    let albums = load_albums(tx, library)?;

    // Step 1: the kept key of each album with present tracks, and the loose tracks.
    let mut members: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    let mut loose: Vec<usize> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        match row.album.filter(|a| albums.contains_key(a)) {
            Some(album) => members.entry(album).or_default().push(i),
            None => loose.push(i),
        }
    }
    // Album to (kept key, tracks in the kept part).
    let mut kept: BTreeMap<i64, (GroupKey, usize)> = BTreeMap::new();
    for (album, idxs) in &members {
        let mut parts: BTreeMap<&GroupKey, usize> = BTreeMap::new();
        for &i in idxs {
            *parts.entry(&keys[i]).or_default() += 1;
        }
        let (best, count) = most_frequent_counted(parts);
        let best = best.clone();
        loose.extend(idxs.iter().copied().filter(|&i| keys[i] != best));
        kept.insert(*album, (best, count));
    }

    // Step 2: one album per key.
    let mut by_key: BTreeMap<&GroupKey, Vec<i64>> = BTreeMap::new();
    for (album, (key, _)) in &kept {
        by_key.entry(key).or_default().push(*album);
    }
    let mut target: HashMap<GroupKey, i64> = HashMap::new();
    let mut absorbed: Vec<(i64, i64)> = Vec::new();
    let mut touched: BTreeSet<i64> = BTreeSet::new();
    let mut rekeyed: Vec<(i64, GroupKey)> = Vec::new();
    for (key, list) in by_key {
        let winner = *list
            .iter()
            .max_by_key(|&&a| (kept[&a].1, Reverse(albums[&a].created_at), Reverse(a)))
            .expect("a key has at least one album");
        for &other in &list {
            if other != winner {
                absorbed.push((other, winner));
                touched.insert(winner);
            }
        }
        if albums[&winner].group_key != key.as_str() {
            rekeyed.push((winner, key.clone()));
            touched.insert(winner);
        }
        target.insert(key.clone(), winner);
    }

    // Loose tracks go to the album of their key.
    let mut hidden_by_key: HashMap<&str, i64> = HashMap::new();
    let mut hidden: Vec<&AlbumRow> = albums
        .values()
        .filter(|a| !kept.contains_key(&a.key))
        .collect();
    hidden.sort_by_key(|a| (Reverse(a.created_at), Reverse(a.key)));
    for album in hidden {
        // The last insert wins: the oldest album with the key.
        hidden_by_key.insert(album.group_key.as_str(), album.key);
    }
    let mut created: Vec<(i64, LibraryAlbumId)> = Vec::new();
    let mut assignment: Vec<Option<i64>> = rows.iter().map(|r| r.album).collect();
    loose.sort_unstable();
    let mut move_track =
        tx.prepare_cached("UPDATE tracks SET library_album_id = ?2 WHERE id = ?1")?;
    for i in loose {
        let key = &keys[i];
        let album = match target.get(key) {
            Some(album) => *album,
            None => {
                let album = match hidden_by_key.get(key.as_str()) {
                    Some(album) => *album,
                    None => {
                        let (album, id) = insert_album(tx, library, key, now)?;
                        created.push((album, id));
                        album
                    }
                };
                target.insert(key.clone(), album);
                album
            }
        };
        if let Some(old) = assignment[i] {
            touched.insert(old);
        }
        touched.insert(album);
        assignment[i] = Some(album);
        move_track.execute(params![rows[i].key, album])?;
    }
    drop(move_track);

    for (album, key) in &rekeyed {
        tx.prepare_cached("UPDATE library_albums SET group_key = ?2 WHERE id = ?1")?
            .execute(params![album, key.as_str()])?;
    }
    let mut absorbed_ids: Vec<LibraryAlbumId> = Vec::new();
    for (other, winner) in &absorbed {
        merge_into(tx, &albums[other], &albums[winner])?;
        absorbed_ids.push(albums[other].id.clone());
        for a in assignment.iter_mut().filter(|a| **a == Some(*other)) {
            *a = Some(*winner);
        }
    }

    // Step 3: the fields of every album with present tracks.
    let mut final_members: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for (i, album) in assignment.iter().enumerate() {
        if let Some(album) = album {
            final_members.entry(*album).or_default().push(i);
        }
    }
    let stored = load_album_data(tx, library)?;
    let stored_artists = load_album_artists(tx, library)?;
    let ids: HashMap<i64, LibraryAlbumId> = albums
        .values()
        .map(|a| (a.key, a.id.clone()))
        .chain(created.iter().cloned())
        .collect();
    for (album, idxs) in &final_members {
        let key = &keys[idxs[0]];
        let data = album_data(key, &rows, idxs, &credits);
        if stored.get(album) != Some(&data) {
            write_data(tx, *album, &data, now)?;
            touched.insert(*album);
        }
        let rep = idxs
            .iter()
            .copied()
            .find(|&i| credits[i] == data.artist_credit)
            .unwrap_or(idxs[0]);
        let (list, sort) = credit_list(key, &rows[rep], &credits[rep]);
        let wanted: Vec<LinkedArtist> = list.iter().map(LinkedArtist::from_credit).collect();
        if stored_artists.get(album).map_or(&[][..], Vec::as_slice) != wanted.as_slice() {
            link_album_artists(tx, &ids[album], &list, sort)?;
            touched.insert(*album);
        }
    }

    let mut bump = tx.prepare_cached("UPDATE library_albums SET updated_at = ?2 WHERE id = ?1")?;
    let mut out: BTreeSet<LibraryAlbumId> = absorbed_ids.into_iter().collect();
    for album in touched {
        if let Some(id) = ids.get(&album) {
            if !absorbed.iter().any(|(other, _)| *other == album) {
                bump.execute(params![album, now])?;
            }
            out.insert(id.clone());
        }
    }
    Ok(out.into_iter().collect())
}

/// Moves everything of the album `from` into `into` and deletes `from`, leaving a
/// redirect.
fn merge_into(tx: &Transaction<'_>, from: &AlbumRow, into: &AlbumRow) -> Result<()> {
    tx.prepare_cached("UPDATE tracks SET library_album_id = ?2 WHERE library_album_id = ?1")?
        .execute(params![from.key, into.key])?;
    // min() and max() with a NULL argument give NULL: coalesce keeps the other value.
    tx.prepare_cached(
        "INSERT INTO album_annotations (user_id, album_id, starred_at, rating)
         SELECT user_id, ?2, starred_at, rating FROM album_annotations WHERE album_id = ?1
         ON CONFLICT (user_id, album_id) DO UPDATE SET
           starred_at = coalesce(min(album_annotations.starred_at, excluded.starred_at),
                                 album_annotations.starred_at, excluded.starred_at),
           rating = coalesce(max(album_annotations.rating, excluded.rating),
                             album_annotations.rating, excluded.rating)",
    )?
    .execute(params![from.key, into.key])?;
    add_redirect(tx, from.id.as_str(), into.id.as_str())?;
    tx.prepare_cached("DELETE FROM library_albums WHERE id = ?1")?
        .execute([from.key])?;
    Ok(())
}

/// What the album key computations need of a present track.
struct TrackRow {
    key: i64,
    album: Option<i64>,
    folder: String,
    title_norm: Option<String>,
    tags: TagFacts,
    first_artist: Option<FirstArtist>,
}

/// The first artist the credit of a track is linked to.
struct FirstArtist {
    id: ArtistId,
    name: String,
    /// Its MusicBrainz id, when the link came from it (not matched by name).
    mbid: Option<String>,
}

enum Filter<'a> {
    All,
    /// Tracks with this normalized album title.
    Title(&'a str),
    /// One track, by internal key.
    Track(i64),
}

/// Present tracks of `library` selected by `filter`, in key order.
fn load_rows(conn: &Connection, library: LibraryId, filter: Filter<'_>) -> Result<Vec<TrackRow>> {
    let (condition, arg): (&str, rusqlite::types::Value) = match filter {
        Filter::All => ("", rusqlite::types::Value::Null),
        Filter::Title(title) => ("AND t.album_title_norm = ?2", title.to_owned().into()),
        Filter::Track(key) => ("AND t.id = ?2", key.into()),
    };
    let sql = format!(
        "SELECT t.id, t.library_album_id, t.rel_path, t.album_title, t.album_sort,
                t.album_artist_credit, t.album_artist_sort, t.album_artists_tag,
                t.mb_album_artist_ids, t.artist_credit, t.artist_sort, t.artists_tag,
                t.mb_artist_ids, t.compilation, t.mb_release_id, t.mb_release_group_id,
                t.date_y, t.date_m, t.date_d, t.orig_date_y, t.orig_date_m, t.orig_date_d,
                a.public_id, a.name, CASE WHEN ta.matched_by_name = 0 THEN a.mbid END
         FROM tracks t
         LEFT JOIN track_artists ta ON ta.track_id = t.id AND ta.position = 0
         LEFT JOIN artists a ON a.id = ta.artist_id
         WHERE t.library_id = ?1 AND t.state = 'present' {condition}
         ORDER BY t.id"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let map = |r: &rusqlite::Row<'_>| -> rusqlite::Result<TrackRow> {
        let rel_path: String = r.get(2)?;
        let album_title: Option<String> = r.get(3)?;
        let tags = TagFacts {
            album_title: album_title.clone(),
            album_sort: r.get(4)?,
            album_artist_credit: r.get(5)?,
            album_artist_sort: r.get(6)?,
            album_artists: split_values(r.get::<_, Option<String>>(7)?.as_deref()),
            mb_album_artist_ids: split_values(r.get::<_, Option<String>>(8)?.as_deref()),
            artist_credit: r.get(9)?,
            artist_sort: r.get(10)?,
            artists: split_values(r.get::<_, Option<String>>(11)?.as_deref()),
            mb_artist_ids: split_values(r.get::<_, Option<String>>(12)?.as_deref()),
            compilation: r.get(13)?,
            mb_release_id: r.get(14)?,
            mb_release_group_id: r.get(15)?,
            date: date_from(r.get(16)?, r.get(17)?, r.get(18)?),
            original_date: date_from(r.get(19)?, r.get(20)?, r.get(21)?),
            ..TagFacts::default()
        };
        let first_artist = match r.get::<_, Option<ArtistId>>(22)? {
            Some(id) => Some(FirstArtist {
                id,
                name: r.get(23)?,
                mbid: r.get(24)?,
            }),
            None => None,
        };
        Ok(TrackRow {
            key: r.get(0)?,
            album: r.get(1)?,
            folder: album_folder(&rel_path).to_owned(),
            title_norm: album_title.as_deref().map(normalize),
            tags,
            first_artist,
        })
    };
    let rows = match arg {
        rusqlite::types::Value::Null => stmt.query_map(params![library.0], map)?,
        arg => stmt.query_map(params![library.0, arg], map)?,
    };
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The (album folder, normalized album title) pairs of `rows` whose tracks without
/// album artist tag have more than one artist credit (normalized).
fn mixed_groups(rows: &[TrackRow]) -> BTreeSet<(&str, &str)> {
    let mut credits: BTreeMap<(&str, &str), BTreeSet<String>> = BTreeMap::new();
    for row in rows {
        if row.tags.album_artist_credit.is_some() {
            continue;
        }
        if let (Some(title), Some(credit)) = (&row.title_norm, &row.tags.artist_credit) {
            credits
                .entry((row.folder.as_str(), title.as_str()))
                .or_default()
                .insert(norm_name(credit));
        }
    }
    credits
        .into_iter()
        .filter(|(_, set)| set.len() > 1)
        .map(|(group, _)| group)
        .collect()
}

/// The album credit of every row. `rows` must hold every present track of each album
/// folder and title they cover.
fn album_credits(rows: &[TrackRow]) -> Vec<String> {
    let mixed = mixed_groups(rows);
    rows.iter()
        .map(|row| {
            let is_mixed = row
                .title_norm
                .as_deref()
                .is_some_and(|t| mixed.contains(&(row.folder.as_str(), t)));
            album_credit(&row.tags, is_mixed)
        })
        .collect()
}

/// The group key of every row, folder rule included.
fn group_keys(rows: &[TrackRow], credits: &[String]) -> Vec<GroupKey> {
    // Releases named by the tracks of each (album folder, tag key).
    let mut releases: HashMap<(&str, GroupKey), BTreeSet<&str>> = HashMap::new();
    for (row, credit) in rows.iter().zip(credits) {
        if let (Some(release), Some(title)) = (&row.tags.mb_release_id, &row.tags.album_title) {
            releases
                .entry((row.folder.as_str(), tag_key(title, credit)))
                .or_default()
                .insert(release);
        }
    }
    rows.iter()
        .zip(credits)
        .map(|(row, credit)| {
            if let (None, Some(title)) = (&row.tags.mb_release_id, &row.tags.album_title) {
                let key = tag_key(title, credit);
                return match releases.get(&(row.folder.as_str(), key.clone())) {
                    Some(set) if set.len() == 1 => {
                        GroupKey(format!("mb:{}", set.iter().next().expect("one release")))
                    }
                    _ => key,
                };
            }
            group_key(&row.tags, credit, row.first_artist.as_ref().map(|a| &a.id))
        })
        .collect()
}

/// The fields of a library album computed from its tracks.
#[derive(Clone, Debug, PartialEq, Eq)]
struct AlbumData {
    title: String,
    sort_title: Option<String>,
    artist_credit: String,
    release_mbid: Option<String>,
    release_group_mbid: Option<String>,
    is_singles: bool,
    date: Option<PartialDate>,
    original_date: Option<PartialDate>,
}

/// The fields of the album with key `key` and tracks `rows[idxs]`.
fn album_data(key: &GroupKey, rows: &[TrackRow], idxs: &[usize], credits: &[String]) -> AlbumData {
    let tracks = || idxs.iter().map(|&i| &rows[i]);
    if key.is_singles() {
        return AlbumData {
            title: SINGLES_TITLE.to_owned(),
            sort_title: None,
            artist_credit: tracks()
                .find_map(|r| r.first_artist.as_ref().map(|a| a.name.clone()))
                .unwrap_or_default(),
            release_mbid: None,
            release_group_mbid: most_frequent(
                tracks().filter_map(|r| r.tags.mb_release_group_id.clone()),
            ),
            is_singles: true,
            date: most_frequent(tracks().filter_map(|r| r.tags.date)),
            original_date: most_frequent(tracks().filter_map(|r| r.tags.original_date)),
        };
    }
    AlbumData {
        title: most_frequent(tracks().filter_map(|r| r.tags.album_title.clone()))
            .unwrap_or_default(),
        sort_title: most_frequent(tracks().filter_map(|r| r.tags.album_sort.clone())),
        artist_credit: most_frequent(idxs.iter().map(|&i| credits[i].clone())).unwrap_or_default(),
        release_mbid: key.release().map(str::to_owned),
        release_group_mbid: most_frequent(
            tracks().filter_map(|r| r.tags.mb_release_group_id.clone()),
        ),
        is_singles: false,
        date: most_frequent(tracks().filter_map(|r| r.tags.date)),
        original_date: most_frequent(tracks().filter_map(|r| r.tags.original_date)),
    }
}

/// The most frequent value; ties go to the smallest (the oldest, for dates).
fn most_frequent<T: Ord>(values: impl Iterator<Item = T>) -> Option<T> {
    let mut counts: BTreeMap<T, usize> = BTreeMap::new();
    for v in values {
        *counts.entry(v).or_default() += 1;
    }
    (!counts.is_empty()).then(|| most_frequent_counted(counts).0)
}

/// The entry with the highest count of a non-empty map; ties go to the smallest key.
fn most_frequent_counted<T: Ord>(counts: BTreeMap<T, usize>) -> (T, usize) {
    let mut best: Option<(T, usize)> = None;
    for (value, count) in counts {
        if best.as_ref().is_none_or(|(_, n)| count > *n) {
            best = Some((value, count));
        }
    }
    best.expect("a non-empty map")
}

/// The artists of an album with key `key` credited `credit`, split from the tags of the
/// track `row`, and the sort name to store for them.
fn credit_list<'a>(
    key: &GroupKey,
    row: &'a TrackRow,
    credit: &str,
) -> (Vec<CreditArtist>, Option<&'a str>) {
    let t = &row.tags;
    if key.is_singles() {
        let list = row
            .first_artist
            .iter()
            .map(|a| CreditArtist {
                credited_name: a.name.clone(),
                join_phrase: String::new(),
                mbid: a.mbid.clone(),
            })
            .collect();
        return (list, None);
    }
    if let Some(album_artist) = &t.album_artist_credit {
        let list = split_credit(album_artist, &t.album_artists, &t.mb_album_artist_ids);
        return (list, t.album_artist_sort.as_deref());
    }
    if credit == VARIOUS_ARTISTS && t.artist_credit.as_deref() != Some(VARIOUS_ARTISTS) {
        return (split_credit(VARIOUS_ARTISTS, &[], &[]), None);
    }
    let list = split_credit(
        t.artist_credit.as_deref().unwrap_or(""),
        &t.artists,
        &t.mb_artist_ids,
    );
    (list, t.artist_sort.as_deref())
}

/// One stored album artist, to compare with a credit without writing.
#[derive(Debug, PartialEq, Eq)]
struct LinkedArtist {
    credited_name: String,
    join_phrase: String,
    /// The MusicBrainz id of the credit (`None` for a link matched by name).
    mbid: Option<String>,
}

impl LinkedArtist {
    fn from_credit(c: &CreditArtist) -> LinkedArtist {
        LinkedArtist {
            credited_name: c.credited_name.clone(),
            join_phrase: c.join_phrase.clone(),
            mbid: c.mbid.clone(),
        }
    }
}

/// An album of the library, for [`reconcile_albums`].
struct AlbumRow {
    key: i64,
    id: LibraryAlbumId,
    group_key: String,
    created_at: i64,
}

fn load_albums(conn: &Connection, library: LibraryId) -> Result<HashMap<i64, AlbumRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, public_id, group_key, created_at FROM library_albums WHERE library_id = ?1",
    )?;
    let rows = stmt.query_map([library.0], |r| {
        Ok(AlbumRow {
            key: r.get(0)?,
            id: r.get(1)?,
            group_key: r.get(2)?,
            created_at: r.get(3)?,
        })
    })?;
    rows.map(|r| Ok(r.map(|a| (a.key, a))?)).collect()
}

fn load_album_data(conn: &Connection, library: LibraryId) -> Result<HashMap<i64, AlbumData>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, title, sort_title, artist_credit, release_mbid, release_group_mbid,
                is_singles, date_y, date_m, date_d, orig_date_y, orig_date_m, orig_date_d
         FROM library_albums WHERE library_id = ?1",
    )?;
    let rows = stmt.query_map([library.0], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            AlbumData {
                title: r.get(1)?,
                sort_title: r.get(2)?,
                artist_credit: r.get(3)?,
                release_mbid: r.get(4)?,
                release_group_mbid: r.get(5)?,
                is_singles: r.get(6)?,
                date: date_from(r.get(7)?, r.get(8)?, r.get(9)?),
                original_date: date_from(r.get(10)?, r.get(11)?, r.get(12)?),
            },
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn load_album_artists(
    conn: &Connection,
    library: LibraryId,
) -> Result<HashMap<i64, Vec<LinkedArtist>>> {
    let mut stmt = conn.prepare_cached(
        "SELECT l.album_id, l.credited_name, l.join_phrase,
                CASE WHEN l.matched_by_name = 0 THEN a.mbid END
         FROM album_artists l
         JOIN library_albums al ON al.id = l.album_id
         JOIN artists a ON a.id = l.artist_id
         WHERE al.library_id = ?1 ORDER BY l.album_id, l.position",
    )?;
    let mut out: HashMap<i64, Vec<LinkedArtist>> = HashMap::new();
    let mut rows = stmt.query([library.0])?;
    while let Some(r) = rows.next()? {
        out.entry(r.get(0)?).or_default().push(LinkedArtist {
            credited_name: r.get(1)?,
            join_phrase: r.get(2)?,
            mbid: r.get(3)?,
        });
    }
    Ok(out)
}

fn insert_album(
    tx: &Transaction<'_>,
    library: LibraryId,
    key: &GroupKey,
    now: i64,
) -> Result<(i64, LibraryAlbumId)> {
    let id = LibraryAlbumId::generate();
    tx.prepare_cached(
        "INSERT INTO library_albums (public_id, library_id, group_key, title, artist_credit,
                                     is_singles, created_at, updated_at)
         VALUES (?1, ?2, ?3, '', '', ?4, ?5, ?5)",
    )?
    .execute(params![id, library.0, key.as_str(), key.is_singles(), now])?;
    Ok((tx.last_insert_rowid(), id))
}

fn write_data(tx: &Transaction<'_>, album: i64, data: &AlbumData, now: i64) -> Result<()> {
    let (date_y, date_m, date_d) = date_parts(data.date);
    let (orig_y, orig_m, orig_d) = date_parts(data.original_date);
    tx.prepare_cached(
        "UPDATE library_albums SET title = ?2, sort_title = ?3, artist_credit = ?4,
           release_mbid = ?5, release_group_mbid = ?6, is_singles = ?7,
           date_y = ?8, date_m = ?9, date_d = ?10,
           orig_date_y = ?11, orig_date_m = ?12, orig_date_d = ?13, updated_at = ?14
         WHERE id = ?1",
    )?
    .execute(params![
        album,
        data.title,
        data.sort_title,
        data.artist_credit,
        data.release_mbid,
        data.release_group_mbid,
        data.is_singles,
        date_y,
        date_m,
        date_d,
        orig_y,
        orig_m,
        orig_d,
        now
    ])?;
    Ok(())
}

fn date_parts(date: Option<PartialDate>) -> (Option<i32>, Option<u8>, Option<u8>) {
    date.map_or((None, None, None), |d| (Some(d.year), d.month, d.day))
}

fn date_from(year: Option<i32>, month: Option<u8>, day: Option<u8>) -> Option<PartialDate> {
    year.map(|year| PartialDate { year, month, day })
}
