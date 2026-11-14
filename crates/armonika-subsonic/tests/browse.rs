mod common;

use armonika_testdata::fixture::FixtureTags;
use common::*;
use serde_json::Value;

fn ann() -> String {
    login("ann", ANN_PASSWORD)
}

fn admin() -> String {
    login("admin", ADMIN_PASSWORD)
}

/// A track of `artist` (single artist) on `album`.
fn song(title: &str, artist: &str, album: &str, no: u32) -> FixtureTags {
    FixtureTags {
        title: Some(title.to_owned()),
        artist_credit: Some(artist.to_owned()),
        artists: vec![artist.to_owned()],
        album_title: Some(album.to_owned()),
        album_artist_credit: Some(artist.to_owned()),
        album_artists: vec![artist.to_owned()],
        track_no: Some(no),
        ..FixtureTags::default()
    }
}

async fn call(app: &App, method: &str, login: &str, extra: &str) -> Value {
    get(app, &format!("/rest/{method}?{login}&{extra}"))
        .await
        .json()
}

fn titles(songs: &Value) -> Vec<String> {
    songs
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["title"].as_str().unwrap().to_owned())
        .collect()
}

fn names(items: &Value) -> Vec<String> {
    items
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap().to_owned())
        .collect()
}

/// Every artist of a `getArtists` answer, in index order.
fn all_artists(v: &Value) -> Vec<Value> {
    v["artists"]["index"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|i| i["artist"].as_array().unwrap().clone())
        .collect()
}

/// The id of the album named `name` in the list of all albums of `login`.
async fn album_id(app: &App, login: &str, name: &str) -> String {
    let v = call(
        app,
        "getAlbumList2",
        login,
        "type=alphabeticalByName&size=500",
    )
    .await;
    v["albumList2"]["album"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == name)
        .unwrap_or_else(|| panic!("no album {name}"))["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn artists_indexed_ignoring_articles() {
    let app = app_empty().await;
    app.put(
        &app.music_a,
        "1.mp3",
        &song("S1", "The Beatles", "Abbey", 1),
    );
    app.put(&app.music_a, "2.mp3", &song("S2", "Zebra", "Stripes", 1));
    app.put(&app.music_a, "3.mp3", &song("S3", "Cat", "Meow", 1));
    app.scan();
    let v = call(&app, "getArtists", &ann(), "").await;
    let artists = &v["artists"];
    assert_eq!(
        artists["ignoredArticles"],
        "The El La Los Las Le Les Il Lo Gli I"
    );
    let index = artists["index"].as_array().unwrap();
    let letters: Vec<&str> = index.iter().map(|i| i["name"].as_str().unwrap()).collect();
    assert_eq!(letters, ["B", "C", "Z"]);
    assert_eq!(names(&index[0]["artist"]), ["The Beatles"]);
    assert_eq!(index[0]["artist"][0]["albumCount"], 1);
}

#[tokio::test]
async fn album_tracks_in_disc_track_order() {
    let app = app_empty().await;
    for (rel, title, disc, no) in [
        ("a.mp3", "D2T1", 2, 1),
        ("b.mp3", "D1T2", 1, 2),
        ("c.mp3", "D1T1", 1, 1),
    ] {
        let mut t = song(title, "Band", "Double", no);
        t.disc_no = Some(disc);
        app.put(&app.music_a, rel, &t);
    }
    app.scan();
    let id = album_id(&app, &ann(), "Double").await;
    let v = call(&app, "getAlbum", &ann(), &format!("id={id}")).await;
    assert_eq!(titles(&v["album"]["song"]), ["D1T1", "D1T2", "D2T1"]);
    assert_eq!(v["album"]["songCount"], 3);
    let first = &v["album"]["song"][0];
    assert_eq!(first["parent"], id);
    assert_eq!(first["albumId"], id);
    assert_eq!(first["isDir"], false);
    assert_eq!(first["type"], "music");
    assert_eq!(first["discNumber"], 1);
    assert_eq!(first["track"], 1);
    assert_eq!(first["suffix"], "mp3");
    assert_eq!(first["contentType"], "audio/mpeg");
    assert_eq!(first["path"], "c.mp3");
}

#[tokio::test]
async fn missing_track_not_listed_and_get_song_is_70() {
    let app = app_empty().await;
    app.put(&app.music_a, "1.mp3", &song("Keep", "Band", "LP", 1));
    app.put(&app.music_a, "2.mp3", &song("Gone", "Band", "LP", 2));
    app.scan();
    let id = album_id(&app, &ann(), "LP").await;
    let v = call(&app, "getAlbum", &ann(), &format!("id={id}")).await;
    let gone = v["album"]["song"][1]["id"].as_str().unwrap().to_owned();
    let v = call(&app, "getSong", &ann(), &format!("id={gone}")).await;
    assert_eq!(v["status"], "ok");
    assert_eq!(v["song"]["title"], "Gone");

    std::fs::remove_file(app.music_a.join("2.mp3")).unwrap();
    app.scan();
    let v = call(&app, "getAlbum", &ann(), &format!("id={id}")).await;
    assert_eq!(titles(&v["album"]["song"]), ["Keep"]);
    assert_eq!(v["album"]["songCount"], 1);
    let v = call(&app, "getSong", &ann(), &format!("id={gone}")).await;
    assert_eq!(v["status"], "failed");
    assert_eq!(v["error"]["code"], 70);
}

#[tokio::test]
async fn other_library_album_is_70() {
    let app = app_empty().await;
    app.put(&app.music_a, "1.mp3", &song("Mine", "Ann Band", "Open", 1));
    app.put(
        &app.music_b,
        "1.mp3",
        &song("Secret", "Hidden", "Closed", 1),
    );
    app.scan();
    let id = album_id(&app, &admin(), "Closed").await;
    let v = call(&app, "getAlbum", &ann(), &format!("id={id}")).await;
    assert_eq!(v["error"]["code"], 70);
    let v = call(&app, "getAlbum", &admin(), &format!("id={id}")).await;
    assert_eq!(v["status"], "ok");
    let song_id = v["album"]["song"][0]["id"].as_str().unwrap().to_owned();
    let v = call(&app, "getSong", &ann(), &format!("id={song_id}")).await;
    assert_eq!(v["error"]["code"], 70);

    // An artist that exists only in a library the user cannot reach is not listed and
    // cannot be fetched.
    let v = call(&app, "getArtists", &ann(), "").await;
    let listed: Vec<String> = all_artists(&v)
        .iter()
        .map(|a| a["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(listed, ["Ann Band"]);
    let v = call(&app, "getArtists", &admin(), "").await;
    let hidden = all_artists(&v)
        .into_iter()
        .find(|a| a["name"] == "Hidden")
        .unwrap();
    let hidden_id = hidden["id"].as_str().unwrap();
    let v = call(&app, "getArtist", &ann(), &format!("id={hidden_id}")).await;
    assert_eq!(v["error"]["code"], 70);
}

#[tokio::test]
async fn music_folder_filter_applies() {
    let app = app_empty().await;
    app.put(&app.music_a, "1.mp3", &song("A1", "Band A", "Alpha", 1));
    app.put(&app.music_b, "1.mp3", &song("B1", "Band B", "Beta", 1));
    app.scan();
    let list = async |login: String, extra: String| {
        let v = call(
            &app,
            "getAlbumList2",
            &login,
            &format!("type=newest&{extra}"),
        )
        .await;
        names(&v["albumList2"]["album"])
    };
    let mut both = list(admin(), String::new()).await;
    both.sort();
    assert_eq!(both, ["Alpha", "Beta"]);
    let only_a = list(admin(), format!("musicFolderId={}", app.lib_a.0)).await;
    assert_eq!(only_a, ["Alpha"]);
    let only_b = list(admin(), format!("musicFolderId={}", app.lib_b.0)).await;
    assert_eq!(only_b, ["Beta"]);
    let none = list(ann(), format!("musicFolderId={}", app.lib_b.0)).await;
    assert!(none.is_empty());
    let extra = format!("musicFolderId={}", app.lib_b.0);
    let v = call(&app, "getRandomSongs", &ann(), &extra).await;
    assert!(
        v["randomSongs"]
            .get("song")
            .is_none_or(|s| s.as_array().unwrap().is_empty())
    );
}

#[tokio::test]
async fn album_list_starred_is_per_user() {
    let app = app_empty().await;
    app.put(&app.music_a, "1.mp3", &song("S", "Band", "Loved", 1));
    app.put(&app.music_a, "2.mp3", &song("T", "Band", "Plain", 1));
    app.scan();
    app.exec(
        "INSERT INTO album_annotations (user_id, album_id, starred_at, rating)
         SELECT u.id, al.id, 1700000000000, 4 FROM users u, library_albums al
         WHERE u.name = 'admin' AND al.title = 'Loved'",
    )
    .await;
    let v = call(&app, "getAlbumList2", &admin(), "type=starred").await;
    assert_eq!(names(&v["albumList2"]["album"]), ["Loved"]);
    assert_eq!(v["albumList2"]["album"][0]["userRating"], 4);
    assert_eq!(
        v["albumList2"]["album"][0]["starred"],
        "2023-11-14T22:13:20.000Z"
    );
    let v = call(&app, "getAlbumList2", &ann(), "type=starred").await;
    assert!(
        v["albumList2"]
            .get("album")
            .is_none_or(|a| a.as_array().unwrap().is_empty())
    );
    let v = call(&app, "getAlbumList2", &ann(), "type=highest").await;
    assert!(
        v["albumList2"]
            .get("album")
            .is_none_or(|a| a.as_array().unwrap().is_empty())
    );
    let id = album_id(&app, &ann(), "Loved").await;
    let v = call(&app, "getAlbum", &ann(), &format!("id={id}")).await;
    assert!(v["album"].get("starred").is_none());
    assert!(v["album"].get("userRating").is_none());
}

#[tokio::test]
async fn album_list_paging_is_stable() {
    let app = app_empty().await;
    for (i, name) in ["Echo", "Alpha", "Delta", "Bravo", "Charlie"]
        .iter()
        .enumerate()
    {
        app.put(
            &app.music_a,
            &format!("{i}.mp3"),
            &song("S", "Band", name, 1),
        );
    }
    app.scan();
    for kind in ["alphabeticalByName", "newest"] {
        let all = call(
            &app,
            "getAlbumList2",
            &ann(),
            &format!("type={kind}&size=10"),
        )
        .await;
        let all = names(&all["albumList2"]["album"]);
        assert_eq!(all.len(), 5);
        let mut paged = Vec::new();
        for offset in [0, 2, 4] {
            let v = call(
                &app,
                "getAlbumList2",
                &ann(),
                &format!("type={kind}&size=2&offset={offset}"),
            )
            .await;
            paged.extend(names(&v["albumList2"]["album"]));
        }
        assert_eq!(paged, all, "{kind}");
    }
    let v = call(&app, "getAlbumList2", &ann(), "type=alphabeticalByName").await;
    assert_eq!(
        names(&v["albumList2"]["album"]),
        ["Alpha", "Bravo", "Charlie", "Delta", "Echo"]
    );
    let v = call(&app, "getAlbumList2", &ann(), "type=random&size=2").await;
    assert_eq!(v["albumList2"]["album"].as_array().unwrap().len(), 2);
    let v = call(&app, "getAlbumList2", &ann(), "").await;
    assert_eq!(v["error"]["code"], 10);
}

#[tokio::test]
async fn random_songs_respects_size() {
    let app = app_empty().await;
    for n in 0..5 {
        let title = format!("S{n}");
        app.put(
            &app.music_a,
            &format!("{n}.mp3"),
            &song(&title, "Band", "LP", n + 1),
        );
    }
    app.scan();
    let v = call(&app, "getRandomSongs", &ann(), "size=3").await;
    assert_eq!(v["randomSongs"]["song"].as_array().unwrap().len(), 3);
    let v = call(&app, "getRandomSongs", &ann(), "").await;
    assert_eq!(v["randomSongs"]["song"].as_array().unwrap().len(), 5);
    let v = call(&app, "getRandomSongs", &ann(), "size=1000").await;
    assert_eq!(v["randomSongs"]["song"].as_array().unwrap().len(), 5);
}

#[tokio::test]
async fn directory_browsing_walks_artist_album_tracks() {
    let app = app_empty().await;
    app.put(&app.music_a, "1.mp3", &song("One", "Band", "LP", 1));
    app.put(&app.music_a, "2.mp3", &song("Two", "Band", "LP", 2));
    app.scan();
    let v = call(&app, "getIndexes", &ann(), "").await;
    let indexes = &v["indexes"];
    assert!(indexes["lastModified"].as_i64().unwrap() > 0);
    let artist = &indexes["index"][0]["artist"][0];
    assert_eq!(artist["name"], "Band");
    let artist_id = artist["id"].as_str().unwrap();

    let v = call(
        &app,
        "getMusicDirectory",
        &ann(),
        &format!("id={artist_id}"),
    )
    .await;
    let dir = &v["directory"];
    assert_eq!(dir["id"], artist_id);
    assert_eq!(dir["name"], "Band");
    let child = &dir["child"][0];
    assert_eq!(child["isDir"], true);
    assert_eq!(child["title"], "LP");
    assert_eq!(child["parent"], artist_id);
    let album_id = child["id"].as_str().unwrap();

    let v = call(&app, "getMusicDirectory", &ann(), &format!("id={album_id}")).await;
    let dir = &v["directory"];
    assert_eq!(dir["name"], "LP");
    assert_eq!(titles(&dir["child"]), ["One", "Two"]);
    assert_eq!(dir["child"][0]["isDir"], false);
    assert_eq!(dir["child"][0]["parent"], album_id);

    let song_id = dir["child"][0]["id"].as_str().unwrap();
    let v = call(&app, "getMusicDirectory", &ann(), &format!("id={song_id}")).await;
    assert_eq!(v["error"]["code"], 70);
    let v = call(&app, "getMusicDirectory", &ann(), "id=ar-missing").await;
    assert_eq!(v["error"]["code"], 70);
}

#[tokio::test]
async fn multi_artist_track_appears_under_each_artist() {
    let app = app_empty().await;
    let mut t = song("Duet", "A feat. B", "Together", 1);
    t.artists = vec!["A".into(), "B".into()];
    t.album_artist_credit = Some("A feat. B".into());
    t.album_artists = vec!["A".into(), "B".into()];
    app.put(&app.music_a, "1.mp3", &t);
    app.scan();
    let v = call(&app, "getArtists", &ann(), "").await;
    let artists = all_artists(&v);
    assert_eq!(names(&Value::Array(artists.clone())), ["A", "B"]);
    for a in &artists {
        let extra = format!("id={}", a["id"].as_str().unwrap());
        let v = call(&app, "getArtist", &ann(), &extra).await;
        assert_eq!(names(&v["artist"]["album"]), ["Together"]);
        assert_eq!(v["artist"]["albumCount"], 1);
    }
    let id = album_id(&app, &ann(), "Together").await;
    let v = call(&app, "getAlbum", &ann(), &format!("id={id}")).await;
    let song = &v["album"]["song"][0];
    assert_eq!(song["artist"], "A feat. B");
    assert_eq!(song["displayArtist"], "A feat. B");
    assert_eq!(names(&song["artists"]), ["A", "B"]);
    assert_eq!(names(&song["albumArtists"]), ["A", "B"]);
    assert_eq!(song["artistId"], artists[0]["id"]);
    assert_eq!(v["album"]["artistId"], artists[0]["id"]);
}

#[tokio::test]
async fn vinyl_sides_in_natural_order() {
    let app = app_empty().await;
    for (i, text) in ["B1", "A10", "A2", "A1"].iter().enumerate() {
        let mut t = song(text, "Band", "Vinyl", 0);
        t.track_no = None;
        t.track_number_text = Some((*text).to_owned());
        app.put(&app.music_a, &format!("{i}.mp3"), &t);
    }
    app.scan();
    let id = album_id(&app, &ann(), "Vinyl").await;
    let v = call(&app, "getAlbum", &ann(), &format!("id={id}")).await;
    assert_eq!(titles(&v["album"]["song"]), ["A1", "A2", "A10", "B1"]);
    assert!(v["album"]["song"][0].get("track").is_none());
}

#[tokio::test]
async fn sort_tag_orders_artist() {
    let app = app_empty().await;
    let mut bowie = song("Heroes", "David Bowie", "Heroes", 1);
    bowie.artist_sort = Some("Bowie, David".into());
    bowie.album_artist_sort = Some("Bowie, David".into());
    app.put(&app.music_a, "1.mp3", &bowie);
    app.put(&app.music_a, "2.mp3", &song("S", "Cat Stevens", "Tea", 1));
    app.put(&app.music_a, "3.mp3", &song("S", "Aardvark", "Ants", 1));
    app.scan();
    let v = call(&app, "getArtists", &ann(), "").await;
    let index = v["artists"]["index"].as_array().unwrap();
    let letters: Vec<&str> = index.iter().map(|i| i["name"].as_str().unwrap()).collect();
    assert_eq!(letters, ["A", "B", "C"]);
    assert_eq!(names(&index[1]["artist"]), ["David Bowie"]);
    assert_eq!(index[1]["artist"][0]["sortName"], "Bowie, David");
}

#[tokio::test]
async fn remaster_sorted_by_original_year() {
    let app = app_empty().await;
    let mut remaster = song("S", "Band", "Remaster", 1);
    remaster.date = Some("2011".into());
    remaster.original_date = Some("1975".into());
    let mut later = song("S", "Band", "Later", 1);
    later.date = Some("1990".into());
    let mut early = song("S", "Band", "Early", 1);
    early.date = Some("1970".into());
    let mut late = song("S", "Band", "Late", 1);
    late.date = Some("2005".into());
    for (i, t) in [remaster, later, early, late].iter().enumerate() {
        app.put(&app.music_a, &format!("{i}.mp3"), t);
    }
    app.scan();
    let q = "type=byYear&fromYear=1970&toYear=2000";
    let v = call(&app, "getAlbumList2", &ann(), q).await;
    let list = &v["albumList2"]["album"];
    assert_eq!(names(list), ["Early", "Remaster", "Later"]);
    assert_eq!(list[1]["year"], 1975);
    let q = "type=byYear&fromYear=2000&toYear=1970";
    let v = call(&app, "getAlbumList2", &ann(), q).await;
    assert_eq!(
        names(&v["albumList2"]["album"]),
        ["Later", "Remaster", "Early"]
    );
}

#[tokio::test]
async fn album_exposes_release_and_original_dates() {
    let app = app_empty().await;
    let mut t = song("S", "Band", "Dated", 1);
    t.date = Some("1975-10-03".into());
    t.original_date = Some("1973-05".into());
    t.disc_no = Some(1);
    t.disc_subtitle = Some("Side one".into());
    t.compilation = true;
    app.put(&app.music_a, "1.mp3", &t);
    app.scan();
    let id = album_id(&app, &ann(), "Dated").await;
    let v = call(&app, "getAlbum", &ann(), &format!("id={id}")).await;
    let album = &v["album"];
    assert_eq!(
        album["releaseDate"],
        serde_json::json!({"year": 1975, "month": 10, "day": 3})
    );
    assert_eq!(
        album["originalReleaseDate"],
        serde_json::json!({"year": 1973, "month": 5})
    );
    assert_eq!(album["year"], 1973);
    assert_eq!(
        album["discTitles"],
        serde_json::json!([{"disc": 1, "title": "Side one"}])
    );
    assert_eq!(album["isCompilation"], true);
    assert_eq!(album["song"][0]["year"], 1975);

    // XML carries the same dates as child elements.
    let x = get(
        &app,
        &format!(
            "/rest/getAlbum?{}&id={id}",
            ann().replace("f=json", "f=xml")
        ),
    )
    .await;
    let doc = roxmltree::Document::parse(&x.body).unwrap();
    let release = doc
        .descendants()
        .find(|n| n.tag_name().name() == "originalReleaseDate")
        .unwrap();
    assert_eq!(release.attribute("year"), Some("1973"));
    assert_eq!(release.attribute("month"), Some("5"));
    assert_eq!(release.attribute("day"), None);
}

#[tokio::test]
async fn merged_album_old_id_returns_kept_album() {
    let app = app_empty().await;
    app.put(&app.music_a, "1.mp3", &song("S", "Band", "Kept", 1));
    app.scan();
    let id = album_id(&app, &ann(), "Kept").await;
    let kept = id.clone();
    app.db
        .write(move |c| {
            let tx = c.transaction()?;
            armonika_core::ids::add_redirect(&tx, "al-0ldid", &kept)?;
            Ok(tx.commit()?)
        })
        .await
        .unwrap();
    let v = call(&app, "getAlbum", &ann(), "id=al-0ldid").await;
    assert_eq!(v["status"], "ok");
    assert_eq!(v["album"]["name"], "Kept");
    assert_eq!(v["album"]["id"], id);
}

#[tokio::test]
async fn indexes_if_modified_since_returns_empty_when_unchanged() {
    let app = app_empty().await;
    app.put(&app.music_a, "1.mp3", &song("S", "Band", "LP", 1));
    app.scan();
    let v = call(&app, "getIndexes", &ann(), "").await;
    let last = v["indexes"]["lastModified"].as_i64().unwrap();
    assert!(!v["indexes"]["index"].as_array().unwrap().is_empty());
    let v = call(
        &app,
        "getIndexes",
        &ann(),
        &format!("ifModifiedSince={last}"),
    )
    .await;
    assert_eq!(v["status"], "ok");
    assert_eq!(v["indexes"]["lastModified"].as_i64().unwrap(), last);
    assert!(v["indexes"].get("index").is_none());
    let older = last - 1;
    let v = call(
        &app,
        "getIndexes",
        &ann(),
        &format!("ifModifiedSince={older}"),
    )
    .await;
    assert!(!v["indexes"]["index"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn genres_and_info_endpoints() {
    let app = app_empty().await;
    let mut t = song("S", "Band", "LP", 1);
    t.genres = vec!["Rock".into()];
    app.put(&app.music_a, "1.mp3", &t);
    let mut t = song("T", "Band", "LP", 2);
    t.genres = vec!["rock".into(), "Pop".into()];
    app.put(&app.music_a, "2.mp3", &t);
    app.scan();
    let v = call(&app, "getGenres", &ann(), "").await;
    let genres = v["genres"]["genre"].as_array().unwrap();
    assert_eq!(genres.len(), 2);
    assert_eq!(genres[0]["value"], "Pop");
    assert_eq!(genres[1]["songCount"], 2);
    assert_eq!(genres[1]["albumCount"], 1);
    let v = call(
        &app,
        "getSongsByGenre",
        &ann(),
        "genre=rock&count=1&offset=1",
    )
    .await;
    assert_eq!(v["songsByGenre"]["song"].as_array().unwrap().len(), 1);
    let v = call(&app, "getSongsByGenre", &ann(), "").await;
    assert_eq!(v["error"]["code"], 10);
    let v = call(&app, "getAlbumList2", &ann(), "type=byGenre&genre=Pop").await;
    assert_eq!(names(&v["albumList2"]["album"]), ["LP"]);
    let v = call(&app, "getAlbumList", &ann(), "type=newest").await;
    assert_eq!(v["albumList"]["album"][0]["isDir"], true);
    for (method, node) in [
        ("getTopSongs", "topSongs"),
        ("getSimilarSongs2", "similarSongs2"),
    ] {
        let v = call(&app, method, &ann(), "id=x&artist=Band").await;
        assert_eq!(v["status"], "ok", "{method}");
        assert!(v[node]["song"].as_array().unwrap().is_empty(), "{method}");
    }
    let id = album_id(&app, &ann(), "LP").await;
    for (method, node) in [
        ("getAlbumInfo2", "albumInfo"),
        ("getArtistInfo2", "artistInfo2"),
    ] {
        let v = call(&app, method, &ann(), &format!("id={id}")).await;
        assert_eq!(v["status"], "ok", "{method}");
        assert!(v.get(node).is_some(), "{method}");
    }
}

#[tokio::test]
async fn artist_with_only_missing_tracks_is_not_listed() {
    let app = app_empty().await;
    app.put(&app.music_a, "1.mp3", &song("Stay", "Staying", "One", 1));
    app.put(&app.music_a, "2.mp3", &song("Go", "Leaving", "Two", 1));
    app.scan();
    std::fs::remove_file(app.music_a.join("2.mp3")).unwrap();
    app.scan();
    let v = call(&app, "getArtists", &ann(), "").await;
    assert_eq!(names(&Value::Array(all_artists(&v))), ["Staying"]);
}
