//! Read-only queries for browsing: tracks, library albums, artists and genres as the
//! apps see them.
//!
//! Every query shows only tracks in state `present` that belong to the libraries of the
//! [`Scope`]; an album is visible when one of its present tracks is, an artist when it
//! is linked to a visible track or album. An empty list of libraries gives no results.
//! The views carry the annotations (favorite, rating, plays) of the user of the scope,
//! never those of another user. Identifiers that were merged resolve to the survivor.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fmt::Write;

use rusqlite::{Connection, ToSql, params_from_iter};

use crate::Result;
use crate::ids::{
    ArtistId, LibraryAlbumId, LibraryId, TrackId, UserId, resolve_album, resolve_artist,
};
use crate::model::PartialDate;
use crate::text::fold_sort;
use crate::users::require_user;

/// What a query may see and for whom: the libraries, the user whose annotations are
/// shown and the articles ignored when sorting names ("The", "L'").
#[derive(Clone, Copy, Debug)]
pub struct Scope<'a> {
    pub libs: &'a [LibraryId],
    pub user: &'a UserId,
    pub articles: &'a [String],
}

impl Scope<'_> {
    /// The libraries as the body of an `IN (...)` list (integers, so safe to inline).
    fn lib_list(&self) -> String {
        let mut out = String::new();
        for (i, lib) in self.libs.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            let _ = write!(out, "{}", lib.0);
        }
        out
    }

    fn user_key(&self, conn: &Connection) -> Result<i64> {
        require_user(conn, self.user)
    }

    /// Sort key of a name without an explicit sort value: article dropped.
    fn name_key(&self, name: &str) -> String {
        let articles: Vec<&str> = self.articles.iter().map(String::as_str).collect();
        fold_sort(name, &articles)
    }

    /// Sort key of a name: the explicit sort value if there is one, else the name.
    fn sort_key(&self, sort: Option<&str>, name: &str) -> String {
        match sort.filter(|s| !s.trim().is_empty()) {
            Some(s) => fold_sort(s, &[]),
            None => self.name_key(name),
        }
    }
}

/// What the user of the scope did with an item.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Annotations {
    /// When the item was starred, Unix milliseconds.
    pub starred_at: Option<i64>,
    /// Rating from 1 to 5.
    pub rating: Option<u8>,
    pub play_count: u64,
    /// When it was last played, Unix milliseconds.
    pub last_played_at: Option<i64>,
}

/// An artist with the identifier and the name used by the apps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedArtist {
    pub id: ArtistId,
    pub name: String,
    pub sort_name: Option<String>,
}

/// A track as the apps see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackView {
    key: i64,
    pub id: TrackId,
    pub library: LibraryId,
    pub album_id: Option<LibraryAlbumId>,
    /// Title of the album (of the library album, else the tag).
    pub album: Option<String>,
    /// The tag, or the file name without extension if the file has none.
    pub title: String,
    pub sort_title: Option<String>,
    /// Name sort key, articles dropped (or from the explicit sort title).
    pub sort_key: String,
    /// The artist as credited in the file.
    pub artist: String,
    pub artists: Vec<NamedArtist>,
    pub album_artists: Vec<NamedArtist>,
    pub track_no: Option<u32>,
    pub track_number_text: Option<String>,
    pub disc_no: Option<u32>,
    /// Year of this edition.
    pub year: Option<i32>,
    pub genres: Vec<String>,
    pub size: u64,
    pub content_type: String,
    pub suffix: String,
    pub duration_ms: Option<u64>,
    pub bitrate_kbps: Option<u32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    pub bit_depth: Option<u32>,
    /// Path relative to the library root.
    pub path: String,
    /// MusicBrainz recording id.
    pub mbid: Option<String>,
    pub added_at: i64,
    /// Whether the album of the track has cover art.
    pub album_has_cover: bool,
    pub annotations: Annotations,
}

/// A library album as the apps see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlbumView {
    key: i64,
    pub id: LibraryAlbumId,
    pub library: LibraryId,
    pub title: String,
    pub sort_title: Option<String>,
    pub sort_key: String,
    /// Sort key of the first album artist, for ordering by artist.
    pub artist_sort_key: String,
    /// The artist as credited.
    pub artist: String,
    pub artists: Vec<NamedArtist>,
    /// Release date of the edition.
    pub date: Option<PartialDate>,
    /// First release date.
    pub original_date: Option<PartialDate>,
    pub song_count: u32,
    pub duration_ms: u64,
    pub genres: Vec<String>,
    /// Disc number and subtitle.
    pub disc_titles: Vec<(u32, String)>,
    pub is_compilation: bool,
    pub is_singles: bool,
    /// MusicBrainz release id.
    pub mbid: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub annotations: Annotations,
}

impl AlbumView {
    /// The year used to order and show the album: the original one if known, else the
    /// year of the edition, so a remaster sorts with the original album.
    pub fn year(&self) -> Option<i32> {
        self.original_date.or(self.date).map(|d| d.year)
    }
}

/// An artist as the apps see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtistView {
    pub id: ArtistId,
    pub name: String,
    pub sort_name: Option<String>,
    pub sort_key: String,
    pub mbid: Option<String>,
    /// Visible albums of which this artist is an album artist.
    pub album_count: u32,
    pub updated_at: i64,
    pub annotations: Annotations,
}

/// What an album list is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AlbumListKind {
    Random,
    Newest,
    /// Most played by the user.
    Frequent,
    /// Played most recently by the user.
    Recent,
    /// Starred by the user.
    Starred,
    AlphabeticalByName,
    AlphabeticalByArtist,
    /// Years `from` to `to`, descending when `from` is greater.
    ByYear {
        from: i32,
        to: i32,
    },
    ByGenre(String),
    /// Best rated by the user.
    Highest,
}

fn u32_of(v: Option<i64>) -> Option<u32> {
    v.and_then(|v| u32::try_from(v).ok())
}

fn u64_of(v: Option<i64>) -> Option<u64> {
    v.map(i64::unsigned_abs)
}

fn date_of(y: Option<i64>, m: Option<i64>, d: Option<i64>) -> Option<PartialDate> {
    Some(PartialDate {
        year: i32::try_from(y?).ok()?,
        month: m.and_then(|m| u8::try_from(m).ok()),
        day: d.and_then(|d| u8::try_from(d).ok()),
    })
}

fn annotations_of(
    starred: Option<i64>,
    rating: Option<i64>,
    plays: Option<i64>,
    last: Option<i64>,
) -> Annotations {
    Annotations {
        starred_at: starred,
        rating: rating.and_then(|r| u8::try_from(r).ok()),
        play_count: plays.map_or(0, i64::unsigned_abs),
        last_played_at: last,
    }
}

fn file_stem(rel: &str) -> String {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.rsplit_once('.')
        .map_or(name, |(stem, _)| stem)
        .to_owned()
}

fn content_type(format: &str) -> &'static str {
    match format {
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "ogg" => "audio/ogg",
        "opus" => "audio/ogg",
        "m4a" => "audio/mp4",
        "wav" => "audio/x-wav",
        "aiff" => "audio/aiff",
        _ => "application/octet-stream",
    }
}

fn suffix_of(rel: &str, format: &str) -> String {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    match name.rsplit_once('.') {
        Some((_, ext)) if !ext.is_empty() => ext.to_ascii_lowercase(),
        _ => format.to_owned(),
    }
}

// ---------------------------------------------------------------- tracks

const TRACK_SELECT: &str = "SELECT t.id, t.public_id, t.library_id, al.public_id, al.title,
        t.title, t.sort_title, t.artist_credit, t.album_title, t.track_no,
        t.track_number_text, t.disc_no, t.date_y, t.format, t.size, t.duration_ms,
        t.bitrate_kbps, t.sample_rate, t.channels, t.bit_depth, t.rel_path,
        t.mb_recording_id, t.added_at, t.library_album_id,
        EXISTS (SELECT 1 FROM tracks c WHERE c.library_album_id = t.library_album_id
                AND c.state = 'present' AND c.has_embedded_cover = 1),
        ta.starred_at, ta.rating, ta.play_count, ta.last_played_at
     FROM tracks t
     LEFT JOIN library_albums al ON al.id = t.library_album_id
     LEFT JOIN track_annotations ta ON ta.track_id = t.id AND ta.user_id = ";

struct Pending {
    view: TrackView,
    album_key: Option<i64>,
    credit: Option<String>,
}

/// Runs the track select with `tail` (more `WHERE` terms, `ORDER BY`, `LIMIT`) after the
/// scope conditions.
pub(crate) fn load_tracks(
    conn: &Connection,
    scope: &Scope<'_>,
    tail: &str,
    binds: &[&dyn ToSql],
) -> Result<Vec<TrackView>> {
    if scope.libs.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "{TRACK_SELECT}{} WHERE t.state = 'present' AND t.library_id IN ({}) {tail}",
        scope.user_key(conn)?,
        scope.lib_list()
    );
    let mut stmt = conn.prepare(&sql)?;
    let pending = stmt
        .query_map(params_from_iter(binds.iter().copied()), |r| {
            let rel: String = r.get(20)?;
            let format: String = r.get(13)?;
            let tag_title: Option<String> = r.get(5)?;
            let title = tag_title
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| file_stem(&rel));
            let sort_title: Option<String> = r.get(6)?;
            let album_tag: Option<String> = r.get(8)?;
            Ok(Pending {
                album_key: r.get(23)?,
                credit: r.get(7)?,
                view: TrackView {
                    key: r.get(0)?,
                    id: r.get(1)?,
                    library: LibraryId(r.get(2)?),
                    album_id: r.get(3)?,
                    album: r.get::<_, Option<String>>(4)?.or(album_tag),
                    sort_key: scope.sort_key(sort_title.as_deref(), &title),
                    title,
                    sort_title,
                    artist: String::new(),
                    artists: Vec::new(),
                    album_artists: Vec::new(),
                    track_no: u32_of(r.get(9)?),
                    track_number_text: r.get(10)?,
                    disc_no: u32_of(r.get(11)?),
                    year: r
                        .get::<_, Option<i64>>(12)?
                        .and_then(|y| i32::try_from(y).ok()),
                    genres: Vec::new(),
                    size: r.get::<_, i64>(14)?.unsigned_abs(),
                    content_type: content_type(&format).to_owned(),
                    suffix: suffix_of(&rel, &format),
                    duration_ms: u64_of(r.get(15)?),
                    bitrate_kbps: u32_of(r.get(16)?),
                    sample_rate: u32_of(r.get(17)?),
                    channels: u32_of(r.get(18)?),
                    bit_depth: u32_of(r.get(19)?),
                    path: rel,
                    mbid: r.get(21)?,
                    added_at: r.get(22)?,
                    album_has_cover: r.get(24)?,
                    annotations: annotations_of(r.get(25)?, r.get(26)?, r.get(27)?, r.get(28)?),
                },
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    fill_tracks(conn, pending)
}

fn fill_tracks(conn: &Connection, pending: Vec<Pending>) -> Result<Vec<TrackView>> {
    let mut by_album: HashMap<i64, Vec<NamedArtist>> = HashMap::new();
    let mut out = Vec::with_capacity(pending.len());
    for mut p in pending {
        p.view.artists = artists_of(
            conn,
            "SELECT a.public_id, a.name, a.sort_name FROM track_artists l
             JOIN artists a ON a.id = l.artist_id WHERE l.track_id = ?1 ORDER BY l.position",
            p.view.key,
        )?;
        p.view.artist = p
            .credit
            .filter(|c| !c.trim().is_empty())
            .unwrap_or_else(|| {
                p.view
                    .artists
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            });
        if let Some(album) = p.album_key {
            if let Entry::Vacant(e) = by_album.entry(album) {
                e.insert(album_artists_of(conn, album)?);
            }
            p.view.album_artists = by_album[&album].clone();
        }
        p.view.genres = conn
            .prepare_cached("SELECT genre FROM track_genres WHERE track_id = ?1 ORDER BY genre")?
            .query_map([p.view.key], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        out.push(p.view);
    }
    Ok(out)
}

fn artists_of(conn: &Connection, sql: &str, key: i64) -> Result<Vec<NamedArtist>> {
    Ok(conn
        .prepare_cached(sql)?
        .query_map([key], |r| {
            Ok(NamedArtist {
                id: r.get(0)?,
                name: r.get(1)?,
                sort_name: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
}

fn album_artists_of(conn: &Connection, album: i64) -> Result<Vec<NamedArtist>> {
    artists_of(
        conn,
        "SELECT a.public_id, a.name, a.sort_name FROM album_artists l
         JOIN artists a ON a.id = l.artist_id WHERE l.album_id = ?1 ORDER BY l.position",
        album,
    )
}

/// One run of letters and the number after it, for the natural order of "A2" before "A10".
type NaturalChunk = (String, u64);

/// `"A10"` as `[("a", 10)]`: letters and numbers compare separately.
fn natural_key(text: &str) -> Vec<NaturalChunk> {
    let mut out: Vec<NaturalChunk> = Vec::new();
    let mut letters = String::new();
    let mut digits = String::new();
    for c in text.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else {
            if !digits.is_empty() {
                out.push((
                    std::mem::take(&mut letters),
                    digits.parse().unwrap_or(u64::MAX),
                ));
                digits.clear();
            }
            letters.extend(c.to_lowercase());
        }
    }
    if !letters.is_empty() || !digits.is_empty() {
        out.push((letters, digits.parse().unwrap_or(0)));
    }
    out
}

fn album_order(a: &TrackView, b: &TrackView) -> Ordering {
    let key = |t: &TrackView| {
        (
            t.disc_no.unwrap_or(1),
            t.track_no.unwrap_or(u32::MAX),
            natural_key(t.track_number_text.as_deref().unwrap_or("")),
        )
    };
    key(a)
        .cmp(&key(b))
        .then_with(|| a.sort_key.cmp(&b.sort_key))
        .then_with(|| a.id.cmp(&b.id))
}

/// The track `id` (an identifier of a merged track gives the survivor), if the scope
/// can see it.
pub fn track(conn: &Connection, scope: &Scope<'_>, id: &TrackId) -> Result<Option<TrackView>> {
    let Some(key) = crate::ids::resolve_track(conn, id)? else {
        return Ok(None);
    };
    let key: &dyn ToSql = &key;
    Ok(load_tracks(conn, scope, "AND t.id = ?1", &[key])?
        .into_iter()
        .next())
}

/// The album `id` with its tracks in disc order, then track number; without a number
/// the written number in natural order (`A1, A2, A10, B1`), then the title.
pub fn album(
    conn: &Connection,
    scope: &Scope<'_>,
    id: &LibraryAlbumId,
) -> Result<Option<(AlbumView, Vec<TrackView>)>> {
    let Some(key) = resolve_album(conn, id)? else {
        return Ok(None);
    };
    let filter = format!("AND al.id = {key}");
    let Some(view) = query_albums(conn, scope, &filter, "", "", &[])?
        .into_iter()
        .next()
    else {
        return Ok(None);
    };
    let mut tracks = load_tracks(conn, scope, &format!("AND t.library_album_id = {key}"), &[])?;
    tracks.sort_by(album_order);
    Ok(Some((view, tracks)))
}

/// Random tracks, at most `size`, optionally of one genre and between two years.
pub fn random_tracks(
    conn: &Connection,
    scope: &Scope<'_>,
    size: usize,
    genre: Option<&str>,
    from_year: Option<i32>,
    to_year: Option<i32>,
) -> Result<Vec<TrackView>> {
    let mut tail = String::new();
    let mut binds: Vec<&dyn ToSql> = Vec::new();
    if let Some(genre) = genre.as_ref() {
        tail.push_str(GENRE_FILTER);
        binds.push(genre);
    }
    if let Some(from) = from_year {
        let _ = write!(tail, " AND t.date_y >= {from}");
    }
    if let Some(to) = to_year {
        let _ = write!(tail, " AND t.date_y <= {to}");
    }
    let _ = write!(tail, " ORDER BY random() LIMIT {size}");
    load_tracks(conn, scope, &tail, &binds)
}

const GENRE_FILTER: &str = " AND EXISTS (SELECT 1 FROM track_genres g WHERE g.track_id = t.id AND g.genre = ?1 COLLATE NOCASE)";

/// The tracks of `genre`, `size` of them from `offset`, in a stable order.
pub fn tracks_by_genre(
    conn: &Connection,
    scope: &Scope<'_>,
    genre: &str,
    size: usize,
    offset: usize,
) -> Result<Vec<TrackView>> {
    let genre = genre.to_owned();
    let tail = format!("{GENRE_FILTER} ORDER BY t.id LIMIT {size} OFFSET {offset}");
    load_tracks(conn, scope, &tail, &[&genre])
}

/// A page of every visible track, ordered by identifier (stable while paging).
pub fn all_tracks_page(
    conn: &Connection,
    scope: &Scope<'_>,
    offset: usize,
    limit: usize,
) -> Result<Vec<TrackView>> {
    load_tracks(
        conn,
        scope,
        &format!("ORDER BY t.id LIMIT {limit} OFFSET {offset}"),
        &[],
    )
}

/// `?1, ?2, ...` for `n` binds.
fn numbered_placeholders(n: usize) -> String {
    (1..=n)
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// Puts `items` in the order of `ids` (by `key`), dropping the ones not asked for.
fn in_order<T, K: PartialEq>(ids: &[K], mut items: Vec<T>, key: impl Fn(&T) -> &K) -> Vec<T> {
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(pos) = items.iter().position(|i| key(i) == id) {
            out.push(items.swap_remove(pos));
        }
    }
    out
}

/// The tracks `ids` the scope can see, in the order of `ids`. Others are left out.
pub fn tracks_by_ids(
    conn: &Connection,
    scope: &Scope<'_>,
    ids: &[TrackId],
) -> Result<Vec<TrackView>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let texts: Vec<&str> = ids.iter().map(TrackId::as_str).collect();
    let binds: Vec<&dyn ToSql> = texts.iter().map(|t| t as &dyn ToSql).collect();
    let tail = format!("AND t.public_id IN ({})", numbered_placeholders(ids.len()));
    let found = load_tracks(conn, scope, &tail, &binds)?;
    Ok(in_order(ids, found, |t| &t.id))
}

/// The albums `ids` the scope can see, in the order of `ids`. Others are left out.
pub fn albums_by_ids(
    conn: &Connection,
    scope: &Scope<'_>,
    ids: &[LibraryAlbumId],
) -> Result<Vec<AlbumView>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let texts: Vec<&str> = ids.iter().map(LibraryAlbumId::as_str).collect();
    let binds: Vec<&dyn ToSql> = texts.iter().map(|t| t as &dyn ToSql).collect();
    let filter = format!("AND al.public_id IN ({})", numbered_placeholders(ids.len()));
    let found = query_albums(conn, scope, &filter, "", "", &binds)?;
    Ok(in_order(ids, found, |a| &a.id))
}

/// The artists `ids` the scope can see, in the order of `ids`. Others are left out.
pub fn artists_by_ids(
    conn: &Connection,
    scope: &Scope<'_>,
    ids: &[ArtistId],
) -> Result<Vec<ArtistView>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    // Identifiers are validated types; the quotes are doubled all the same.
    let list = ids
        .iter()
        .map(|id| format!("'{}'", id.as_str().replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(",");
    let found = query_artists(conn, scope, &format!("AND a.public_id IN ({list})"))?;
    Ok(in_order(ids, found, |a| &a.id))
}

/// A page of every visible album, ordered by identifier (stable while paging).
pub fn all_albums_page(
    conn: &Connection,
    scope: &Scope<'_>,
    offset: usize,
    limit: usize,
) -> Result<Vec<AlbumView>> {
    query_albums(
        conn,
        scope,
        "",
        "",
        &format!("ORDER BY al.id LIMIT {limit} OFFSET {offset}"),
        &[],
    )
}

/// A page of every visible artist, ordered by public identifier (stable while paging).
pub fn all_artists_page(
    conn: &Connection,
    scope: &Scope<'_>,
    offset: usize,
    limit: usize,
) -> Result<Vec<ArtistView>> {
    let mut all = query_artists(conn, scope, "")?;
    all.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(all.into_iter().skip(offset).take(limit).collect())
}

/// How much a search result of `user` is lifted by what they did with it: `0.5` if
/// starred plus `0.25 * ln(1 + plays)`. Albums count the plays of their tracks and
/// artists only the star. Computed for one item, so a search pays for its candidates only.
pub fn popularity(conn: &Connection, user: i64, item: &crate::ids::ItemRef) -> Result<f32> {
    use crate::ids::ItemRef;
    let (starred, plays): (bool, i64) = match item {
        ItemRef::Track(id) => conn
            .query_row(
                "SELECT COALESCE(ta.starred_at IS NOT NULL, 0), COALESCE(ta.play_count, 0)
                 FROM tracks t JOIN track_annotations ta ON ta.track_id = t.id
                 WHERE t.public_id = ?1 AND ta.user_id = ?2",
                rusqlite::params![id.as_str(), user],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap_or((false, 0)),
        ItemRef::Album(id) => {
            let starred = conn
                .query_row(
                    "SELECT COUNT(*) FROM library_albums al
                     JOIN album_annotations aa ON aa.album_id = al.id
                     WHERE al.public_id = ?1 AND aa.user_id = ?2 AND aa.starred_at IS NOT NULL",
                    rusqlite::params![id.as_str(), user],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0;
            let plays = conn
                .query_row(
                    "SELECT COALESCE(SUM(ta.play_count), 0) FROM library_albums al
                     JOIN tracks t ON t.library_album_id = al.id
                     JOIN track_annotations ta ON ta.track_id = t.id
                     WHERE al.public_id = ?1 AND ta.user_id = ?2",
                    rusqlite::params![id.as_str(), user],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap_or(0);
            (starred, plays)
        }
        ItemRef::Artist(id) => {
            let starred = conn
                .query_row(
                    "SELECT COUNT(*) FROM artists a
                     JOIN artist_annotations an ON an.artist_id = a.id
                     WHERE a.public_id = ?1 AND an.user_id = ?2 AND an.starred_at IS NOT NULL",
                    rusqlite::params![id.as_str(), user],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0;
            (starred, 0)
        }
    };
    let plays = plays.max(0) as f32;
    Ok(if starred { 0.5 } else { 0.0 } + 0.25 * plays.ln_1p())
}

/// Genres with the number of songs and albums, by name without regard to case.
pub fn genres(conn: &Connection, scope: &Scope<'_>) -> Result<Vec<(String, u32, u32)>> {
    if scope.libs.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT MIN(g.genre), COUNT(DISTINCT t.id), COUNT(DISTINCT t.library_album_id)
         FROM track_genres g JOIN tracks t ON t.id = g.track_id
         WHERE t.state = 'present' AND t.library_id IN ({})
         GROUP BY g.genre COLLATE NOCASE ORDER BY MIN(g.genre) COLLATE NOCASE",
        scope.lib_list()
    );
    Ok(conn
        .prepare(&sql)?
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                u32_of(r.get(1)?).unwrap_or(0),
                u32_of(r.get(2)?).unwrap_or(0),
            ))
        })?
        .collect::<rusqlite::Result<_>>()?)
}

// ---------------------------------------------------------------- albums

/// Runs the album select. `filter` adds `WHERE` terms, `having` a `HAVING` clause and
/// `tail` the `ORDER BY` and `LIMIT`.
pub(crate) fn query_albums(
    conn: &Connection,
    scope: &Scope<'_>,
    filter: &str,
    having: &str,
    tail: &str,
    binds: &[&dyn ToSql],
) -> Result<Vec<AlbumView>> {
    if scope.libs.is_empty() {
        return Ok(Vec::new());
    }
    let user = scope.user_key(conn)?;
    let sql = format!(
        "SELECT al.id, al.public_id, al.library_id, al.title, al.sort_title, al.artist_credit,
                al.release_mbid, al.is_singles,
                al.date_y, al.date_m, al.date_d, al.orig_date_y, al.orig_date_m, al.orig_date_d,
                al.created_at, al.updated_at,
                COUNT(t.id), COALESCE(SUM(t.duration_ms), 0), MAX(t.compilation),
                MAX(aa.starred_at), MAX(aa.rating),
                SUM(ta.play_count), MAX(ta.last_played_at)
         FROM library_albums al
         JOIN tracks t ON t.library_album_id = al.id AND t.state = 'present'
         LEFT JOIN album_annotations aa ON aa.album_id = al.id AND aa.user_id = {user}
         LEFT JOIN track_annotations ta ON ta.track_id = t.id AND ta.user_id = {user}
         WHERE al.library_id IN ({}) {filter}
         GROUP BY al.id {having} {tail}",
        scope.lib_list()
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut views = stmt
        .query_map(params_from_iter(binds.iter().copied()), |r| {
            let title: String = r.get(3)?;
            let sort_title: Option<String> = r.get(4)?;
            Ok(AlbumView {
                key: r.get(0)?,
                id: r.get(1)?,
                library: LibraryId(r.get(2)?),
                sort_key: scope.sort_key(sort_title.as_deref(), &title),
                artist_sort_key: String::new(),
                title,
                sort_title,
                artist: r.get(5)?,
                artists: Vec::new(),
                mbid: r.get(6)?,
                is_singles: r.get(7)?,
                date: date_of(r.get(8)?, r.get(9)?, r.get(10)?),
                original_date: date_of(r.get(11)?, r.get(12)?, r.get(13)?),
                created_at: r.get(14)?,
                updated_at: r.get(15)?,
                song_count: u32_of(r.get(16)?).unwrap_or(0),
                duration_ms: r.get::<_, i64>(17)?.unsigned_abs(),
                is_compilation: r.get::<_, Option<i64>>(18)?.unwrap_or(0) != 0,
                genres: Vec::new(),
                disc_titles: Vec::new(),
                annotations: annotations_of(r.get(19)?, r.get(20)?, r.get(21)?, r.get(22)?),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    for view in &mut views {
        view.artists = album_artists_of(conn, view.key)?;
        view.artist_sort_key = match view.artists.first() {
            Some(a) => scope.sort_key(a.sort_name.as_deref(), &a.name),
            None => scope.name_key(&view.artist),
        };
        view.genres = conn
            .prepare_cached(
                "SELECT DISTINCT g.genre FROM track_genres g JOIN tracks t ON t.id = g.track_id
                 WHERE t.library_album_id = ?1 AND t.state = 'present' ORDER BY g.genre",
            )?
            .query_map([view.key], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        view.disc_titles = conn
            .prepare_cached(
                "SELECT DISTINCT COALESCE(disc_no, 1), disc_subtitle FROM tracks
                 WHERE library_album_id = ?1 AND state = 'present'
                   AND disc_subtitle IS NOT NULL AND disc_subtitle <> ''
                 ORDER BY 1",
            )?
            .query_map([view.key], |r| {
                Ok((u32_of(r.get(0)?).unwrap_or(1), r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
    }
    Ok(views)
}

/// `size` albums of `kind` from `offset`. Frequent, recent, starred and highest use the
/// data of the user of the scope. The order is stable between calls (except random).
pub fn album_list(
    conn: &Connection,
    scope: &Scope<'_>,
    kind: &AlbumListKind,
    size: usize,
    offset: usize,
) -> Result<Vec<AlbumView>> {
    let page = format!("LIMIT {size} OFFSET {offset}");
    let sql_ordered = |having: &str, order: &str| {
        query_albums(
            conn,
            scope,
            "",
            having,
            &format!("ORDER BY {order}, al.id {page}"),
            &[],
        )
    };
    let sorted = |mut all: Vec<AlbumView>, cmp: &dyn Fn(&AlbumView, &AlbumView) -> Ordering| {
        all.sort_by(|a, b| cmp(a, b).then_with(|| a.id.cmp(&b.id)));
        all.into_iter().skip(offset).take(size).collect()
    };
    let by_name_then_year = |a: &AlbumView, b: &AlbumView| {
        a.sort_key
            .cmp(&b.sort_key)
            .then_with(|| a.year().cmp(&b.year()))
    };
    match kind {
        AlbumListKind::Random => query_albums(
            conn,
            scope,
            "",
            "",
            &format!("ORDER BY random() {page}"),
            &[],
        ),
        AlbumListKind::Newest => sql_ordered("", "al.created_at DESC"),
        AlbumListKind::Frequent => {
            sql_ordered("HAVING SUM(ta.play_count) > 0", "SUM(ta.play_count) DESC")
        }
        AlbumListKind::Recent => sql_ordered(
            "HAVING MAX(ta.last_played_at) IS NOT NULL",
            "MAX(ta.last_played_at) DESC",
        ),
        AlbumListKind::Starred => sql_ordered(
            "HAVING MAX(aa.starred_at) IS NOT NULL",
            "MAX(aa.starred_at) DESC",
        ),
        AlbumListKind::Highest => {
            sql_ordered("HAVING MAX(aa.rating) IS NOT NULL", "MAX(aa.rating) DESC")
        }
        AlbumListKind::AlphabeticalByName => Ok(sorted(
            query_albums(conn, scope, "", "", "", &[])?,
            &by_name_then_year,
        )),
        AlbumListKind::AlphabeticalByArtist => Ok(sorted(
            query_albums(conn, scope, "", "", "", &[])?,
            &|a, b| {
                a.artist_sort_key
                    .cmp(&b.artist_sort_key)
                    .then_with(|| by_name_then_year(a, b))
            },
        )),
        AlbumListKind::ByYear { from, to } => {
            let (lo, hi) = (*from.min(to), *from.max(to));
            let all: Vec<AlbumView> = query_albums(conn, scope, "", "", "", &[])?
                .into_iter()
                .filter(|a| a.year().is_some_and(|y| (lo..=hi).contains(&y)))
                .collect();
            let descending = from > to;
            Ok(sorted(all, &|a, b| {
                let by_year = a.year().cmp(&b.year());
                let by_year = if descending {
                    by_year.reverse()
                } else {
                    by_year
                };
                by_year.then_with(|| a.sort_key.cmp(&b.sort_key))
            }))
        }
        AlbumListKind::ByGenre(genre) => {
            let filter = "AND EXISTS (SELECT 1 FROM track_genres g
                 JOIN tracks t2 ON t2.id = g.track_id
                 WHERE t2.library_album_id = al.id AND t2.state = 'present'
                   AND g.genre = ?1 COLLATE NOCASE)";
            let genre = genre.clone();
            let all = query_albums(conn, scope, filter, "", "", &[&genre])?;
            Ok(sorted(all, &by_name_then_year))
        }
    }
}

// --------------------------------------------------------------- artists

pub(crate) fn query_artists(
    conn: &Connection,
    scope: &Scope<'_>,
    filter: &str,
) -> Result<Vec<ArtistView>> {
    if scope.libs.is_empty() {
        return Ok(Vec::new());
    }
    let user = scope.user_key(conn)?;
    let libs = scope.lib_list();
    let sql = format!(
        "SELECT * FROM (
           SELECT a.id, a.public_id, a.name, a.sort_name, a.mbid, a.updated_at,
                  (SELECT COUNT(DISTINCT l.album_id) FROM album_artists l
                   JOIN library_albums al ON al.id = l.album_id
                   WHERE l.artist_id = a.id AND al.library_id IN ({libs})
                     AND EXISTS (SELECT 1 FROM tracks t WHERE t.library_album_id = al.id
                                 AND t.state = 'present')) AS album_count,
                  EXISTS (SELECT 1 FROM track_artists l JOIN tracks t ON t.id = l.track_id
                          WHERE l.artist_id = a.id AND t.state = 'present'
                            AND t.library_id IN ({libs})) AS has_tracks,
                  an.starred_at, an.rating
           FROM artists a
           LEFT JOIN artist_annotations an ON an.artist_id = a.id AND an.user_id = {user}
           WHERE 1 = 1 {filter})
         WHERE album_count > 0 OR has_tracks"
    );
    Ok(conn
        .prepare(&sql)?
        .query_map([], |r| {
            let name: String = r.get(2)?;
            let sort_name: Option<String> = r.get(3)?;
            Ok(ArtistView {
                id: r.get(1)?,
                sort_key: scope.sort_key(sort_name.as_deref(), &name),
                name,
                sort_name,
                mbid: r.get(4)?,
                updated_at: r.get(5)?,
                album_count: u32_of(r.get(6)?).unwrap_or(0),
                annotations: annotations_of(r.get(8)?, r.get(9)?, None, None),
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
}

/// Every visible artist, by sort name. An artist linked to nothing the scope can see
/// (a name-only artist whose tracks are missing or in another library) is left out.
pub fn artists(conn: &Connection, scope: &Scope<'_>) -> Result<Vec<ArtistView>> {
    let mut all = query_artists(conn, scope, "")?;
    all.sort_by(|a, b| {
        a.sort_key
            .cmp(&b.sort_key)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(all)
}

/// The artist `id` with the albums it is album artist of, oldest first, if the scope
/// can see the artist.
pub fn artist(
    conn: &Connection,
    scope: &Scope<'_>,
    id: &ArtistId,
) -> Result<Option<(ArtistView, Vec<AlbumView>)>> {
    let Some(key) = resolve_artist(conn, id)? else {
        return Ok(None);
    };
    let Some(view) = query_artists(conn, scope, &format!("AND a.id = {key}"))?
        .into_iter()
        .next()
    else {
        return Ok(None);
    };
    let filter =
        format!("AND al.id IN (SELECT album_id FROM album_artists WHERE artist_id = {key})");
    let mut albums = query_albums(conn, scope, &filter, "", "", &[])?;
    albums.sort_by(|a, b| {
        a.year()
            .cmp(&b.year())
            .then_with(|| a.sort_key.cmp(&b.sort_key))
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(Some((view, albums)))
}

/// When anything the scope sees last changed (Unix milliseconds, 0 if it sees nothing).
pub fn last_modified(conn: &Connection, scope: &Scope<'_>) -> Result<i64> {
    if scope.libs.is_empty() {
        return Ok(0);
    }
    let libs = scope.lib_list();
    let tracks: Option<i64> = conn.query_row(
        &format!(
            "SELECT MAX(updated_at) FROM tracks
             WHERE library_id IN ({libs})"
        ),
        [],
        |r| r.get(0),
    )?;
    let albums: Option<i64> = conn.query_row(
        &format!(
            "SELECT MAX(al.updated_at) FROM library_albums al
             WHERE al.library_id IN ({libs}) AND EXISTS
               (SELECT 1 FROM tracks t WHERE t.library_album_id = al.id AND t.state = 'present')"
        ),
        [],
        |r| r.get(0),
    )?;
    let mut latest = tracks.unwrap_or(0).max(albums.unwrap_or(0));
    for artist in query_artists(conn, scope, "")? {
        latest = latest.max(artist.updated_at);
    }
    Ok(latest)
}
