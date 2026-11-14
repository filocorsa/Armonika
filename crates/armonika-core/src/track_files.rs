//! The link between tracks and their files, as kept by the scanner (spec §8.1).
//!
//! A track has a stable identifier that never depends on its path or tags. The scanner
//! decides which file belongs to which track; these functions read what it needs and
//! write its decisions, so that internal keys stay inside the core. Every write leaves
//! the track `present`, except [`mark_missing`]. The tag columns are a copy of the tags
//! of the file, written only from [`FileFacts`].

use rusqlite::{Connection, Transaction, params};

use crate::db::now_ms;
use crate::ids::{LibraryId, TrackId, resolve_track};
use crate::model::{AudioFormat, FileFacts, PartialDate};
use crate::text::normalize;
use crate::{Error, Result};

/// What the database knows about the file of a track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackFile {
    pub id: TrackId,
    /// Path relative to the library root, in database form (see [`crate::paths`]).
    pub rel_path: String,
    pub size: u64,
    pub mtime_ms: i64,
    /// Version of the tag reader the tags were read with.
    pub tag_schema: u32,
    pub duration_ms: Option<u64>,
    /// When the track became missing, for missing tracks.
    pub missing_since: Option<i64>,
}

/// How the audio content of a track relates to its stored content hash, for
/// [`write_file`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentChange {
    /// The file did not change (only the tags were read again): the hash stays valid.
    Same,
    /// The file changed: the hash is cleared and must be computed again.
    Changed,
    /// The track was relinked to another file by the pre-filter (§11 point 6): the old
    /// hash and path are kept in `prev_content_hash` and `prev_rel_path`, so the
    /// background hashing can confirm or undo the link, and the hash is cleared.
    Relinked,
}

/// The present tracks of `library`.
pub fn present_files(conn: &Connection, library: LibraryId) -> Result<Vec<TrackFile>> {
    files_in_state(conn, library, "present")
}

/// The missing tracks of `library`.
pub fn missing_files(conn: &Connection, library: LibraryId) -> Result<Vec<TrackFile>> {
    files_in_state(conn, library, "missing")
}

fn files_in_state(conn: &Connection, library: LibraryId, state: &str) -> Result<Vec<TrackFile>> {
    let mut stmt = conn.prepare_cached(
        "SELECT public_id, rel_path, size, mtime_ms, tag_schema, duration_ms, missing_since
         FROM tracks WHERE library_id = ?1 AND state = ?2",
    )?;
    let rows = stmt.query_map(params![library.0, state], |r| {
        Ok(TrackFile {
            id: r.get(0)?,
            rel_path: r.get(1)?,
            size: r.get::<_, i64>(2)?.unsigned_abs(),
            mtime_ms: r.get(3)?,
            tag_schema: u32::try_from(r.get::<_, i64>(4)?).unwrap_or(0),
            duration_ms: r.get::<_, Option<i64>>(5)?.map(i64::unsigned_abs),
            missing_since: r.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Creates a present track for the file at `rel_path` with a new identifier and
/// returns it.
pub fn insert_track(
    tx: &Transaction<'_>,
    library: LibraryId,
    rel_path: &str,
    facts: &FileFacts,
    tag_schema: u32,
) -> Result<TrackId> {
    let id = TrackId::generate();
    let now = now_ms();
    tx.prepare_cached(
        "INSERT INTO tracks (public_id, library_id, state, rel_path, size, mtime_ms,
                             tag_schema, format, hash_state, added_at, updated_at)
         VALUES (?1, ?2, 'present', ?3, 0, 0, 0, '', 'pending', ?4, ?4)",
    )?
    .execute(params![id, library.0, rel_path, now])?;
    let key = tx.last_insert_rowid();
    write_columns(tx, key, rel_path, facts, tag_schema, now)?;
    Ok(id)
}

/// Writes what was read from the file at `rel_path` into the track `id`, which
/// becomes present at that path. `change` says what happens to its content hash.
pub fn write_file(
    tx: &Transaction<'_>,
    id: &TrackId,
    rel_path: &str,
    facts: &FileFacts,
    tag_schema: u32,
    change: ContentChange,
) -> Result<()> {
    let key = require(tx, id)?;
    match change {
        ContentChange::Same => {}
        ContentChange::Changed => {
            tx.prepare_cached(
                "UPDATE tracks SET content_hash = NULL, hash_state = 'pending' WHERE id = ?1",
            )?
            .execute([key])?;
        }
        ContentChange::Relinked => {
            // A track relinked twice before being hashed keeps the last hash it had.
            tx.prepare_cached(
                "UPDATE tracks SET prev_content_hash = COALESCE(content_hash, prev_content_hash),
                                   prev_rel_path = rel_path,
                                   content_hash = NULL, hash_state = 'pending'
                 WHERE id = ?1",
            )?
            .execute([key])?;
        }
    }
    write_columns(tx, key, rel_path, facts, tag_schema, now_ms())
}

/// Makes the track `id` present at `rel_path` without reading its file: for a file
/// that came back where it was, or whose name changed only in case, with the same
/// size and modification time.
pub fn set_present_at(tx: &Transaction<'_>, id: &TrackId, rel_path: &str) -> Result<()> {
    let key = require(tx, id)?;
    tx.prepare_cached(
        "UPDATE tracks SET state = 'present', missing_since = NULL, rel_path = ?2,
                           updated_at = ?3
         WHERE id = ?1",
    )?
    .execute(params![key, rel_path, now_ms()])?;
    Ok(())
}

/// Marks the tracks `ids` missing: they are hidden from the apps but keep every link
/// to personal data (favorites, playlists, history).
pub fn mark_missing(tx: &Transaction<'_>, ids: &[TrackId]) -> Result<()> {
    let now = now_ms();
    let mut stmt = tx.prepare_cached(
        "UPDATE tracks SET state = 'missing', missing_since = ?2, updated_at = ?2
         WHERE id = ?1 AND state = 'present'",
    )?;
    for id in ids {
        let key = require(tx, id)?;
        stmt.execute(params![key, now])?;
    }
    Ok(())
}

fn require(conn: &Connection, id: &TrackId) -> Result<i64> {
    resolve_track(conn, id)?.ok_or_else(|| Error::TrackNotFound(id.clone()))
}

fn write_columns(
    tx: &Transaction<'_>,
    key: i64,
    rel_path: &str,
    facts: &FileFacts,
    tag_schema: u32,
    now: i64,
) -> Result<()> {
    let t = &facts.tags;
    let (date_y, date_m, date_d) = date_parts(t.date);
    let (orig_y, orig_m, orig_d) = date_parts(t.original_date);
    tx.prepare_cached(
        "UPDATE tracks SET
           state = 'present', missing_since = NULL, rel_path = ?2, size = ?3, mtime_ms = ?4,
           tag_schema = ?5, format = ?6, duration_ms = ?7, bitrate_kbps = ?8,
           sample_rate = ?9, channels = ?10, bit_depth = ?11,
           title = ?12, sort_title = ?13, artist_credit = ?14, artist_sort = ?15,
           album_title = ?16, album_sort = ?17, album_artist_credit = ?18,
           album_artist_sort = ?19, track_no = ?20, track_number_text = ?21,
           track_total = ?22, disc_no = ?23, disc_total = ?24, disc_subtitle = ?25,
           date_y = ?26, date_m = ?27, date_d = ?28,
           orig_date_y = ?29, orig_date_m = ?30, orig_date_d = ?31,
           compilation = ?32, has_embedded_cover = ?33,
           mb_recording_id = ?34, mb_release_track_id = ?35, mb_release_id = ?36,
           mb_release_group_id = ?37, mb_work_id = ?38, updated_at = ?39,
           artists_tag = ?40, album_artists_tag = ?41, mb_artist_ids = ?42,
           mb_album_artist_ids = ?43, album_title_norm = ?44
         WHERE id = ?1",
    )?
    .execute(params![
        key,
        rel_path,
        i64::try_from(facts.size).unwrap_or(i64::MAX),
        facts.mtime_ms,
        tag_schema,
        format_name(facts.format),
        facts
            .duration_ms
            .map(|d| i64::try_from(d).unwrap_or(i64::MAX)),
        facts.bitrate_kbps,
        facts.sample_rate,
        facts.channels,
        facts.bit_depth,
        t.title,
        t.title_sort,
        t.artist_credit,
        t.artist_sort,
        t.album_title,
        t.album_sort,
        t.album_artist_credit,
        t.album_artist_sort,
        t.track_no,
        t.track_number_text,
        t.track_total,
        t.disc_no,
        t.disc_total,
        t.disc_subtitle,
        date_y,
        date_m,
        date_d,
        orig_y,
        orig_m,
        orig_d,
        t.compilation,
        t.has_embedded_cover,
        t.mb_recording_id,
        t.mb_release_track_id,
        t.mb_release_id,
        t.mb_release_group_id,
        t.mb_work_id,
        now,
        join_values(&t.artists),
        join_values(&t.album_artists),
        join_values(&t.mb_artist_ids),
        join_values(&t.mb_album_artist_ids),
        t.album_title.as_deref().map(normalize),
    ])?;
    tx.prepare_cached("DELETE FROM track_genres WHERE track_id = ?1")?
        .execute([key])?;
    let mut insert =
        tx.prepare_cached("INSERT OR IGNORE INTO track_genres (track_id, genre) VALUES (?1, ?2)")?;
    for genre in &t.genres {
        insert.execute(params![key, genre])?;
    }
    Ok(())
}

/// Separator of the values of a multi-value tag in its text column (U+001F, the ASCII
/// unit separator, which no tag value contains).
const VALUE_SEPARATOR: char = '\u{1f}';

/// The values of a multi-value tag as stored in its column: joined by
/// [`VALUE_SEPARATOR`], `None` when there are none.
fn join_values(values: &[String]) -> Option<String> {
    let joined = values.join(&VALUE_SEPARATOR.to_string());
    (!joined.is_empty()).then_some(joined)
}

/// The values of a multi-value tag column written by [`join_values`].
pub(crate) fn split_values(column: Option<&str>) -> Vec<String> {
    column.map_or_else(Vec::new, |c| {
        c.split(VALUE_SEPARATOR)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .collect()
    })
}

fn date_parts(date: Option<PartialDate>) -> (Option<i32>, Option<u8>, Option<u8>) {
    date.map_or((None, None, None), |d| (Some(d.year), d.month, d.day))
}

/// The value of the `format` column.
fn format_name(format: AudioFormat) -> &'static str {
    match format {
        AudioFormat::Mp3 => "mp3",
        AudioFormat::Flac => "flac",
        AudioFormat::Ogg => "ogg",
        AudioFormat::Opus => "opus",
        AudioFormat::M4a => "m4a",
        AudioFormat::Wav => "wav",
        AudioFormat::Aiff => "aiff",
    }
}
