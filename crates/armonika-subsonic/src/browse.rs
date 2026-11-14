//! Browsing endpoints: `getArtists`, `getIndexes`, `getArtist`, `getAlbum`, `getSong`,
//! `getMusicDirectory`, `getGenres`, the album and song lists, and the info endpoints
//! that answer empty because no external service is used in this phase.
//!
//! Every query goes through the visibility rules of [`armonika_core::tracks`]: a user
//! sees only present tracks of the libraries they can reach, and an item of another
//! library answers 70 exactly as an item that does not exist.

use armonika_core::access::Access;
use armonika_core::db::Connection;
use armonika_core::ids::{ArtistId, ItemRef, LibraryAlbumId, LibraryId, TrackId};
use armonika_core::model::PartialDate;
use armonika_core::tracks::{
    self, AlbumListKind, AlbumView, Annotations, ArtistView, NamedArtist, Scope, TrackView,
};
use serde_json::{Map, Value, json};

use crate::error::ApiError;
use crate::response::Payload;
use crate::{AppState, Params};

/// Largest number of items a list answers.
const MAX_LIST: usize = 500;

/// Items a list answers when `size` or `count` is not given.
const DEFAULT_LIST: usize = 10;

/// Runs `f` on a reader connection with the scope of the caller, narrowed to the music
/// folder `musicFolderId` if one is given.
async fn read<T: Send + 'static>(
    state: &AppState,
    access: &Access,
    params: &Params,
    f: impl FnOnce(&Connection, &Scope<'_>) -> armonika_core::Result<T> + Send + 'static,
) -> Result<T, ApiError> {
    let filter = match params.get("musicFolderId") {
        Some(raw) => Some(
            raw.parse::<LibraryId>()
                .map_err(|_| ApiError::generic("invalid musicFolderId"))?,
        ),
        None => None,
    };
    let libs = access.visible(filter);
    let user = access.user.clone();
    let articles = state.ignored_articles.clone();
    Ok(state
        .db
        .read(move |c| {
            f(
                c,
                &Scope {
                    libs: &libs,
                    user: &user,
                    articles: &articles,
                },
            )
        })
        .await?)
}

/// A number parameter, `default` if absent.
fn number<T: std::str::FromStr>(params: &Params, name: &str, default: T) -> Result<T, ApiError> {
    match params.get(name) {
        None => Ok(default),
        Some(raw) => raw
            .trim()
            .parse()
            .map_err(|_| ApiError::generic(format!("invalid {name}"))),
    }
}

/// A number parameter that must be there.
fn required_number<T: std::str::FromStr>(params: &Params, name: &str) -> Result<T, ApiError> {
    params
        .require(name)?
        .trim()
        .parse()
        .map_err(|_| ApiError::generic(format!("invalid {name}")))
}

/// The page size of a list, at most [`MAX_LIST`].
fn list_size(params: &Params, name: &str) -> Result<usize, ApiError> {
    Ok(number(params, name, DEFAULT_LIST)?.min(MAX_LIST))
}

// ------------------------------------------------------------------ JSON

fn insert(map: &mut Map<String, Value>, key: &str, value: impl Into<Value>) {
    map.insert(key.to_owned(), value.into());
}

fn insert_opt<V: Into<Value>>(map: &mut Map<String, Value>, key: &str, value: Option<V>) {
    if let Some(value) = value {
        map.insert(key.to_owned(), value.into());
    }
}

/// `2023-11-14T22:13:20.000Z` for Unix milliseconds.
fn iso8601(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let in_day = ms.rem_euclid(86_400_000);
    // Days since 1970-01-01 to a calendar date (Howard Hinnant's civil_from_days).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        in_day / 3_600_000,
        in_day / 60_000 % 60,
        in_day / 1000 % 60,
        in_day % 1000
    )
}

fn seconds(ms: u64) -> u64 {
    (ms + 500) / 1000
}

fn date_json(date: PartialDate) -> Value {
    let mut m = Map::new();
    insert(&mut m, "year", date.year);
    insert_opt(&mut m, "month", date.month);
    insert_opt(&mut m, "day", date.day);
    Value::Object(m)
}

fn artist_refs(list: &[NamedArtist]) -> Value {
    Value::Array(
        list.iter()
            .map(|a| json!({ "id": a.id.as_str(), "name": a.name }))
            .collect(),
    )
}

fn annotate(m: &mut Map<String, Value>, a: &Annotations) {
    insert_opt(m, "starred", a.starred_at.map(iso8601));
    insert_opt(m, "userRating", a.rating);
    if a.play_count > 0 {
        insert(m, "playCount", a.play_count);
    }
    insert_opt(m, "played", a.last_played_at.map(iso8601));
}

fn track_json(t: &TrackView) -> Value {
    let mut m = Map::new();
    insert(&mut m, "id", t.id.as_str());
    insert_opt(&mut m, "parent", t.album_id.as_ref().map(|a| a.as_str()));
    insert(&mut m, "title", t.title.as_str());
    insert_opt(&mut m, "album", t.album.as_deref());
    insert(&mut m, "artist", t.artist.as_str());
    insert_opt(&mut m, "track", t.track_no);
    insert_opt(&mut m, "discNumber", t.disc_no);
    insert_opt(&mut m, "year", t.year);
    insert_opt(&mut m, "genre", t.genres.first().map(String::as_str));
    insert_opt(&mut m, "sortName", t.sort_title.as_deref());
    insert(&mut m, "displayArtist", t.artist.as_str());
    let cover = match (&t.album_id, t.album_has_cover) {
        (Some(album), true) => album.as_str(),
        _ => t.id.as_str(),
    };
    insert(&mut m, "coverArt", cover);
    insert(&mut m, "size", t.size);
    insert(&mut m, "contentType", t.content_type.as_str());
    insert(&mut m, "suffix", t.suffix.as_str());
    insert_opt(&mut m, "duration", t.duration_ms.map(seconds));
    insert_opt(&mut m, "bitRate", t.bitrate_kbps);
    insert_opt(&mut m, "samplingRate", t.sample_rate);
    insert_opt(&mut m, "channelCount", t.channels);
    insert_opt(&mut m, "bitDepth", t.bit_depth);
    insert(&mut m, "path", t.path.as_str());
    insert(&mut m, "isDir", false);
    insert(&mut m, "type", "music");
    insert_opt(&mut m, "albumId", t.album_id.as_ref().map(|a| a.as_str()));
    insert_opt(&mut m, "artistId", t.artists.first().map(|a| a.id.as_str()));
    if !t.artists.is_empty() {
        insert(&mut m, "artists", artist_refs(&t.artists));
    }
    if !t.album_artists.is_empty() {
        insert(&mut m, "albumArtists", artist_refs(&t.album_artists));
    }
    insert_opt(&mut m, "musicBrainzId", t.mbid.as_deref());
    insert(&mut m, "created", iso8601(t.added_at));
    annotate(&mut m, &t.annotations);
    Value::Object(m)
}

fn album_json(a: &AlbumView) -> Value {
    let mut m = Map::new();
    insert(&mut m, "id", a.id.as_str());
    insert(&mut m, "name", a.title.as_str());
    insert(&mut m, "artist", a.artist.as_str());
    insert_opt(&mut m, "artistId", a.artists.first().map(|x| x.id.as_str()));
    insert(&mut m, "coverArt", a.id.as_str());
    insert(&mut m, "songCount", a.song_count);
    insert(&mut m, "duration", seconds(a.duration_ms));
    insert(&mut m, "created", iso8601(a.created_at));
    insert_opt(&mut m, "year", a.year());
    insert_opt(&mut m, "genre", a.genres.first().map(String::as_str));
    if !a.genres.is_empty() {
        let list: Vec<Value> = a.genres.iter().map(|g| json!({ "name": g })).collect();
        insert(&mut m, "genres", list);
    }
    annotate(&mut m, &a.annotations);
    if !a.artists.is_empty() {
        insert(&mut m, "artists", artist_refs(&a.artists));
    }
    insert(&mut m, "displayArtist", a.artist.as_str());
    insert(&mut m, "isCompilation", a.is_compilation);
    insert_opt(&mut m, "sortName", a.sort_title.as_deref());
    insert_opt(&mut m, "releaseDate", a.date.map(date_json));
    insert_opt(
        &mut m,
        "originalReleaseDate",
        a.original_date.map(date_json),
    );
    if !a.disc_titles.is_empty() {
        let list: Vec<Value> = a
            .disc_titles
            .iter()
            .map(|(disc, title)| json!({ "disc": disc, "title": title }))
            .collect();
        insert(&mut m, "discTitles", list);
    }
    insert_opt(&mut m, "musicBrainzId", a.mbid.as_deref());
    Value::Object(m)
}

/// An album as an entry of a folder: the same data plus the folder fields.
fn album_dir_json(a: &AlbumView) -> Value {
    let mut v = album_json(a);
    if let Value::Object(m) = &mut v {
        insert(m, "isDir", true);
        insert(m, "title", a.title.as_str());
        insert(m, "album", a.title.as_str());
        insert_opt(m, "parent", a.artists.first().map(|x| x.id.as_str()));
    }
    v
}

fn artist_json(a: &ArtistView) -> Value {
    let mut m = Map::new();
    insert(&mut m, "id", a.id.as_str());
    insert(&mut m, "name", a.name.as_str());
    insert(&mut m, "albumCount", a.album_count);
    annotate(&mut m, &a.annotations);
    insert_opt(&mut m, "sortName", a.sort_name.as_deref());
    insert_opt(&mut m, "musicBrainzId", a.mbid.as_deref());
    Value::Object(m)
}

/// The letter an artist is filed under: the first letter of its sort key, `#` for
/// anything else.
fn index_letter(sort_key: &str) -> String {
    match sort_key.chars().next() {
        Some(c) if c.is_alphabetic() => c.to_uppercase().collect(),
        _ => "#".to_owned(),
    }
}

fn index_json(artists: &[ArtistView]) -> Vec<Value> {
    let mut groups: Vec<(String, Vec<Value>)> = Vec::new();
    for artist in artists {
        let letter = index_letter(&artist.sort_key);
        match groups.last_mut() {
            Some((last, list)) if *last == letter => list.push(artist_json(artist)),
            _ => groups.push((letter, vec![artist_json(artist)])),
        }
    }
    groups
        .into_iter()
        .map(|(name, artist)| json!({ "name": name, "artist": artist }))
        .collect()
}

// -------------------------------------------------------------- endpoints

fn ignored_articles(state: &AppState) -> String {
    state.ignored_articles.join(" ")
}

pub async fn get_artists(
    state: &AppState,
    access: &Access,
    params: &Params,
) -> Result<Payload, ApiError> {
    let (artists, last) = read(state, access, params, |c, scope| {
        Ok((tracks::artists(c, scope)?, tracks::last_modified(c, scope)?))
    })
    .await?;
    Ok(Payload(
        "artists",
        json!({
            "lastModified": last,
            "ignoredArticles": ignored_articles(state),
            "index": index_json(&artists),
        }),
    ))
}

pub async fn get_indexes(
    state: &AppState,
    access: &Access,
    params: &Params,
) -> Result<Payload, ApiError> {
    let since: Option<i64> = match params.get("ifModifiedSince") {
        Some(_) => Some(number(params, "ifModifiedSince", 0)?),
        None => None,
    };
    let (artists, last) = read(state, access, params, move |c, scope| {
        let last = tracks::last_modified(c, scope)?;
        // Nothing changed since the client's copy: skip the work of listing.
        if since.is_some_and(|since| last <= since) {
            return Ok((Vec::new(), last));
        }
        Ok((tracks::artists(c, scope)?, last))
    })
    .await?;
    let mut node = Map::new();
    insert(&mut node, "lastModified", last);
    insert(&mut node, "ignoredArticles", ignored_articles(state));
    if since.is_none_or(|since| last > since) {
        insert(&mut node, "index", index_json(&artists));
    }
    Ok(Payload("indexes", Value::Object(node)))
}

pub async fn get_artist(
    state: &AppState,
    access: &Access,
    params: &Params,
) -> Result<Payload, ApiError> {
    let id: ArtistId = params
        .require("id")?
        .parse()
        .map_err(|_| ApiError::not_found())?;
    let (artist, albums) = read(state, access, params, move |c, scope| {
        tracks::artist(c, scope, &id)
    })
    .await?
    .ok_or_else(ApiError::not_found)?;
    let mut v = artist_json(&artist);
    if let Value::Object(m) = &mut v {
        insert(
            m,
            "album",
            albums.iter().map(album_json).collect::<Vec<_>>(),
        );
    }
    Ok(Payload("artist", v))
}

pub async fn get_album(
    state: &AppState,
    access: &Access,
    params: &Params,
) -> Result<Payload, ApiError> {
    let id: LibraryAlbumId = params
        .require("id")?
        .parse()
        .map_err(|_| ApiError::not_found())?;
    let (album, songs) = read(state, access, params, move |c, scope| {
        tracks::album(c, scope, &id)
    })
    .await?
    .ok_or_else(ApiError::not_found)?;
    let mut v = album_json(&album);
    if let Value::Object(m) = &mut v {
        insert(m, "song", songs.iter().map(track_json).collect::<Vec<_>>());
    }
    Ok(Payload("album", v))
}

pub async fn get_song(
    state: &AppState,
    access: &Access,
    params: &Params,
) -> Result<Payload, ApiError> {
    let id: TrackId = params
        .require("id")?
        .parse()
        .map_err(|_| ApiError::not_found())?;
    let track = read(state, access, params, move |c, scope| {
        tracks::track(c, scope, &id)
    })
    .await?
    .ok_or_else(ApiError::not_found)?;
    Ok(Payload("song", track_json(&track)))
}

/// The folder view, as Navidrome simulates it: an artist holds its albums, an album its
/// tracks.
pub async fn get_music_directory(
    state: &AppState,
    access: &Access,
    params: &Params,
) -> Result<Payload, ApiError> {
    let item: ItemRef = params
        .require("id")?
        .parse()
        .map_err(|_| ApiError::not_found())?;
    let directory = match item {
        ItemRef::Artist(id) => {
            let (artist, albums) = read(state, access, params, move |c, scope| {
                tracks::artist(c, scope, &id)
            })
            .await?
            .ok_or_else(ApiError::not_found)?;
            let mut m = Map::new();
            insert(&mut m, "id", artist.id.as_str());
            insert(&mut m, "name", artist.name.as_str());
            annotate(&mut m, &artist.annotations);
            insert(
                &mut m,
                "child",
                albums.iter().map(album_dir_json).collect::<Vec<_>>(),
            );
            m
        }
        ItemRef::Album(id) => {
            let (album, songs) = read(state, access, params, move |c, scope| {
                tracks::album(c, scope, &id)
            })
            .await?
            .ok_or_else(ApiError::not_found)?;
            let mut m = Map::new();
            insert(&mut m, "id", album.id.as_str());
            insert(&mut m, "name", album.title.as_str());
            insert_opt(
                &mut m,
                "parent",
                album.artists.first().map(|a| a.id.as_str()),
            );
            annotate(&mut m, &album.annotations);
            insert(
                &mut m,
                "child",
                songs.iter().map(track_json).collect::<Vec<_>>(),
            );
            m
        }
        ItemRef::Track(_) => return Err(ApiError::not_found()),
    };
    Ok(Payload("directory", Value::Object(directory)))
}

pub async fn get_genres(
    state: &AppState,
    access: &Access,
    params: &Params,
) -> Result<Payload, ApiError> {
    let genres = read(state, access, params, tracks::genres).await?;
    let list: Vec<Value> = genres
        .into_iter()
        .map(|(name, songs, albums)| {
            json!({ "value": name, "songCount": songs, "albumCount": albums })
        })
        .collect();
    Ok(Payload("genres", json!({ "genre": list })))
}

fn album_kind(params: &Params) -> Result<AlbumListKind, ApiError> {
    let kind = match params.require("type")? {
        "random" => AlbumListKind::Random,
        "newest" => AlbumListKind::Newest,
        "frequent" => AlbumListKind::Frequent,
        "recent" => AlbumListKind::Recent,
        "starred" => AlbumListKind::Starred,
        "alphabeticalByName" => AlbumListKind::AlphabeticalByName,
        "alphabeticalByArtist" => AlbumListKind::AlphabeticalByArtist,
        "highest" => AlbumListKind::Highest,
        "byYear" => AlbumListKind::ByYear {
            from: required_number(params, "fromYear")?,
            to: required_number(params, "toYear")?,
        },
        "byGenre" => AlbumListKind::ByGenre(params.require("genre")?.to_owned()),
        other => return Err(ApiError::generic(format!("unknown list type {other}"))),
    };
    Ok(kind)
}

async fn album_list(
    state: &AppState,
    access: &Access,
    params: &Params,
) -> Result<Vec<AlbumView>, ApiError> {
    let kind = album_kind(params)?;
    let size = list_size(params, "size")?;
    let offset: usize = number(params, "offset", 0)?;
    read(state, access, params, move |c, scope| {
        tracks::album_list(c, scope, &kind, size, offset)
    })
    .await
}

pub async fn get_album_list2(
    state: &AppState,
    access: &Access,
    params: &Params,
) -> Result<Payload, ApiError> {
    let albums = album_list(state, access, params).await?;
    let list: Vec<Value> = albums.iter().map(album_json).collect();
    Ok(Payload("albumList2", json!({ "album": list })))
}

pub async fn get_album_list(
    state: &AppState,
    access: &Access,
    params: &Params,
) -> Result<Payload, ApiError> {
    let albums = album_list(state, access, params).await?;
    let list: Vec<Value> = albums.iter().map(album_dir_json).collect();
    Ok(Payload("albumList", json!({ "album": list })))
}

fn songs_payload(name: &'static str, songs: &[TrackView]) -> Payload {
    let list: Vec<Value> = songs.iter().map(track_json).collect();
    Payload(name, json!({ "song": list }))
}

pub async fn get_random_songs(
    state: &AppState,
    access: &Access,
    params: &Params,
) -> Result<Payload, ApiError> {
    let size = list_size(params, "size")?;
    let genre = params.get("genre").map(str::to_owned);
    let from: Option<i32> = params
        .get("fromYear")
        .map(|_| number(params, "fromYear", 0))
        .transpose()?;
    let to: Option<i32> = params
        .get("toYear")
        .map(|_| number(params, "toYear", 0))
        .transpose()?;
    let songs = read(state, access, params, move |c, scope| {
        tracks::random_tracks(c, scope, size, genre.as_deref(), from, to)
    })
    .await?;
    Ok(songs_payload("randomSongs", &songs))
}

pub async fn get_songs_by_genre(
    state: &AppState,
    access: &Access,
    params: &Params,
) -> Result<Payload, ApiError> {
    let genre = params.require("genre")?.to_owned();
    let size = list_size(params, "count")?;
    let offset: usize = number(params, "offset", 0)?;
    let songs = read(state, access, params, move |c, scope| {
        tracks::tracks_by_genre(c, scope, &genre, size, offset)
    })
    .await?;
    Ok(songs_payload("songsByGenre", &songs))
}

/// `getArtistInfo2`: valid and empty, no external service is used in this phase.
pub fn get_artist_info2(params: &Params) -> Result<Payload, ApiError> {
    params.require("id")?;
    Ok(Payload("artistInfo2", json!({})))
}

/// `getAlbumInfo2`: valid and empty, no external service is used in this phase.
pub fn get_album_info2(params: &Params) -> Result<Payload, ApiError> {
    params.require("id")?;
    Ok(Payload("albumInfo", json!({})))
}

/// `getTopSongs`: an empty list, there is no external source of popularity.
pub fn get_top_songs() -> Payload {
    songs_payload("topSongs", &[])
}

/// `getSimilarSongs2`: an empty list, there is no external source of similarity.
pub fn get_similar_songs2() -> Payload {
    songs_payload("similarSongs2", &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso8601_formats_unix_milliseconds() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso8601(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
        // A leap day.
        assert_eq!(iso8601(1_709_164_800_000), "2024-02-29T00:00:00.000Z");
    }

    #[test]
    fn index_letter_uses_the_first_letter_or_hash() {
        assert_eq!(index_letter("beatles"), "B");
        assert_eq!(index_letter("2pac"), "#");
        assert_eq!(index_letter(""), "#");
    }
}
