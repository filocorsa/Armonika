//! What the search index is built from: the visible tracks, albums and artists of the
//! database, read by public identifier, and the settings row that records the version
//! of the index schema.
//!
//! Visible means: a track in state `present`; an album with at least one present track;
//! an artist linked to a present track or to a visible album. The functions look
//! identifiers up as they are (no redirect), so an identifier that no longer names a
//! row, or names a hidden one, is simply absent from the result.

use std::collections::BTreeMap;

use rusqlite::{Connection, OptionalExtension, params, params_from_iter};

use crate::Result;
use crate::ids::{ArtistId, LibraryAlbumId, LibraryId, TrackId};

/// Largest number of identifiers in one `IN (...)` list.
const CHUNK: usize = 500;

/// A visible track as the index needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexTrack {
    pub id: TrackId,
    pub library: LibraryId,
    pub title: String,
    pub artist_credit: String,
    pub album_title: String,
}

/// A visible album as the index needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexAlbum {
    pub id: LibraryAlbumId,
    pub library: LibraryId,
    pub title: String,
    pub artist_credit: String,
}

/// A visible artist and the libraries where it is visible.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexArtist {
    pub id: ArtistId,
    pub name: String,
    pub libraries: Vec<LibraryId>,
}

fn placeholders(n: usize) -> String {
    vec!["?"; n].join(",")
}

fn file_stem(rel: &str) -> String {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.rsplit_once('.')
        .map_or(name, |(stem, _)| stem)
        .to_owned()
}

/// The present tracks among `ids`. A track without title is named after its file.
pub fn present_tracks(conn: &Connection, ids: &[TrackId]) -> Result<Vec<IndexTrack>> {
    let mut out = Vec::new();
    for chunk in ids.chunks(CHUNK) {
        let sql = format!(
            "SELECT public_id, library_id, title, artist_credit, album_title, rel_path
             FROM tracks WHERE state = 'present' AND public_id IN ({})",
            placeholders(chunk.len())
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(chunk), |r| {
            let title: Option<String> = r.get(2)?;
            let rel: String = r.get(5)?;
            Ok(IndexTrack {
                id: r.get(0)?,
                library: LibraryId(r.get(1)?),
                title: title
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| file_stem(&rel)),
                artist_credit: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                album_title: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
            })
        })?;
        for row in rows {
            out.push(row?);
        }
    }
    Ok(out)
}

const VISIBLE_ALBUM: &str = "EXISTS (SELECT 1 FROM tracks t
     WHERE t.library_album_id = al.id AND t.state = 'present')";

/// The visible albums among `ids`.
pub fn visible_albums(conn: &Connection, ids: &[LibraryAlbumId]) -> Result<Vec<IndexAlbum>> {
    let mut out = Vec::new();
    for chunk in ids.chunks(CHUNK) {
        let sql = format!(
            "SELECT al.public_id, al.library_id, al.title, al.artist_credit
             FROM library_albums al WHERE {VISIBLE_ALBUM} AND al.public_id IN ({})",
            placeholders(chunk.len())
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(chunk), |r| {
            Ok(IndexAlbum {
                id: r.get(0)?,
                library: LibraryId(r.get(1)?),
                title: r.get(2)?,
                artist_credit: r.get(3)?,
            })
        })?;
        for row in rows {
            out.push(row?);
        }
    }
    Ok(out)
}

/// The visible artists among `ids`, with the libraries where each is visible.
/// An artist linked to nothing visible is absent.
pub fn visible_artists(conn: &Connection, ids: &[ArtistId]) -> Result<Vec<IndexArtist>> {
    let mut out = Vec::new();
    for chunk in ids.chunks(CHUNK) {
        let sql = format!(
            "SELECT a.public_id, a.name, v.library_id FROM artists a
             JOIN (SELECT l.artist_id, t.library_id FROM track_artists l
                     JOIN tracks t ON t.id = l.track_id WHERE t.state = 'present'
                   UNION
                   SELECT l.artist_id, al.library_id FROM album_artists l
                     JOIN library_albums al ON al.id = l.album_id WHERE {VISIBLE_ALBUM}) v
               ON v.artist_id = a.id
             WHERE a.public_id IN ({}) ORDER BY a.id, v.library_id",
            placeholders(chunk.len())
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut by_artist: BTreeMap<String, IndexArtist> = BTreeMap::new();
        let mut rows = stmt.query(params_from_iter(chunk))?;
        while let Some(r) = rows.next()? {
            let id: ArtistId = r.get(0)?;
            let name: String = r.get(1)?;
            let library = LibraryId(r.get(2)?);
            by_artist
                .entry(id.to_string())
                .or_insert_with(|| IndexArtist {
                    id,
                    name,
                    libraries: Vec::new(),
                })
                .libraries
                .push(library);
        }
        out.extend(by_artist.into_values());
    }
    Ok(out)
}

/// Identifiers of every present track.
pub fn all_present_track_ids(conn: &Connection) -> Result<Vec<TrackId>> {
    Ok(conn
        .prepare("SELECT public_id FROM tracks WHERE state = 'present'")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

/// Identifiers of every visible album.
pub fn all_visible_album_ids(conn: &Connection) -> Result<Vec<LibraryAlbumId>> {
    let sql = format!("SELECT al.public_id FROM library_albums al WHERE {VISIBLE_ALBUM}");
    Ok(conn
        .prepare(&sql)?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

/// Identifiers of every artist linked to something visible.
pub fn all_visible_artist_ids(conn: &Connection) -> Result<Vec<ArtistId>> {
    let sql = format!(
        "SELECT a.public_id FROM artists a WHERE
           EXISTS (SELECT 1 FROM track_artists l JOIN tracks t ON t.id = l.track_id
                   WHERE l.artist_id = a.id AND t.state = 'present')
        OR EXISTS (SELECT 1 FROM album_artists l JOIN library_albums al ON al.id = l.album_id
                   WHERE l.artist_id = a.id AND {VISIBLE_ALBUM})"
    );
    Ok(conn
        .prepare(&sql)?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

/// The value of the settings row `key`.
pub fn setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
            r.get(0)
        })
        .optional()?)
}

/// Sets the settings row `key`.
pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}
