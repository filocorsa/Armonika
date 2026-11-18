//! Audio fingerprints, keyed by the content hash of the track's audio: copies of the
//! same audio share one row. A file that cannot be fingerprinted is recorded as a
//! failure, so that it is not tried again on every pass.

use rusqlite::{Connection, OptionalExtension, params};

use crate::Result;
use crate::db::now_ms;
use crate::ids::LibraryId;

/// A present track that has its content hash and no fingerprint (and no failure) yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FingerprintJob {
    /// Internal key of the track.
    pub key: i64,
    pub library: LibraryId,
    pub rel_path: String,
    /// Size and modification time the scan saw.
    pub size: u64,
    pub mtime_ms: i64,
    pub content_hash: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredFingerprint {
    pub fingerprint: String,
    /// Name of the decoder that produced the audio.
    pub decoder: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FingerprintState {
    Done(StoredFingerprint),
    /// The reason it failed.
    Failed(String),
    Pending,
}

/// Present tracks of a library by what became of their fingerprint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FingerprintCounts {
    pub done: u64,
    pub failed: u64,
    pub pending: u64,
}

/// The next tracks to fingerprint after the internal key `after`, in key order, at most
/// `limit`, leaving out the libraries in `paused`. Only present tracks whose content
/// hash is computed are listed.
pub fn pending(
    conn: &Connection,
    after: i64,
    paused: &[LibraryId],
    limit: usize,
) -> Result<Vec<FingerprintJob>> {
    let excluded: Vec<String> = paused.iter().map(|l| l.0.to_string()).collect();
    let filter = if excluded.is_empty() {
        String::new()
    } else {
        format!("AND t.library_id NOT IN ({})", excluded.join(","))
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT t.id, t.library_id, t.rel_path, t.size, t.mtime_ms, t.content_hash
         FROM tracks t
         WHERE t.state = 'present' AND t.hash_state = 'done' AND t.content_hash IS NOT NULL
           AND t.id > ?1 {filter}
           AND NOT EXISTS (SELECT 1 FROM audio_fingerprints f WHERE f.content_hash = t.content_hash)
           AND NOT EXISTS (SELECT 1 FROM audio_fingerprint_failures x WHERE x.content_hash = t.content_hash)
         ORDER BY t.id LIMIT ?2"
    ))?;
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let rows = stmt.query_map(params![after, limit], |r| {
        Ok(FingerprintJob {
            key: r.get(0)?,
            library: LibraryId(r.get(1)?),
            rel_path: r.get(2)?,
            size: r.get::<_, i64>(3)?.unsigned_abs(),
            mtime_ms: r.get(4)?,
            content_hash: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// What is known about the fingerprint of the audio with `content_hash`.
pub fn state(conn: &Connection, content_hash: &[u8]) -> Result<FingerprintState> {
    let done = conn
        .query_row(
            "SELECT fingerprint, decoder FROM audio_fingerprints WHERE content_hash = ?1",
            [content_hash],
            |r| {
                Ok(StoredFingerprint {
                    fingerprint: r.get(0)?,
                    decoder: r.get(1)?,
                })
            },
        )
        .optional()?;
    if let Some(done) = done {
        return Ok(FingerprintState::Done(done));
    }
    let failed = conn
        .query_row(
            "SELECT reason FROM audio_fingerprint_failures WHERE content_hash = ?1",
            [content_hash],
            |r| r.get(0),
        )
        .optional()?;
    Ok(failed.map_or(FingerprintState::Pending, FingerprintState::Failed))
}

/// Stores a fingerprint, replacing the failure recorded for the same audio, if any.
pub fn store(conn: &Connection, content_hash: &[u8], fp: &str, decoder: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM audio_fingerprint_failures WHERE content_hash = ?1",
        [content_hash],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO audio_fingerprints (content_hash, fingerprint, decoder, computed_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![content_hash, fp, decoder, now_ms()],
    )?;
    Ok(())
}

/// Records that the audio with `content_hash` cannot be fingerprinted. A fingerprint
/// already stored for it is kept.
pub fn store_failure(conn: &Connection, content_hash: &[u8], reason: &str) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO audio_fingerprint_failures (content_hash, reason, failed_at)
         SELECT ?1, ?2, ?3
         WHERE NOT EXISTS (SELECT 1 FROM audio_fingerprints WHERE content_hash = ?1)",
        params![content_hash, reason, now_ms()],
    )?;
    Ok(())
}

/// The present tracks of `library` with a computed content hash, by fingerprint state.
pub fn counts(conn: &Connection, library: LibraryId) -> Result<FingerprintCounts> {
    let (done, failed, pending) = conn.query_row(
        "SELECT
           COALESCE(SUM(f.content_hash IS NOT NULL), 0),
           COALESCE(SUM(f.content_hash IS NULL AND x.content_hash IS NOT NULL), 0),
           COALESCE(SUM(f.content_hash IS NULL AND x.content_hash IS NULL), 0)
         FROM tracks t
         LEFT JOIN audio_fingerprints f ON f.content_hash = t.content_hash
         LEFT JOIN audio_fingerprint_failures x ON x.content_hash = t.content_hash
         WHERE t.library_id = ?1 AND t.state = 'present' AND t.hash_state = 'done'
           AND t.content_hash IS NOT NULL",
        [library.0],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        },
    )?;
    Ok(FingerprintCounts {
        done: done.unsigned_abs(),
        failed: failed.unsigned_abs(),
        pending: pending.unsigned_abs(),
    })
}
