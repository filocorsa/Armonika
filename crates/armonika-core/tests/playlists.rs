use std::collections::HashMap;

use armonika_core::access::{Access, Permission};
use armonika_core::db::Db;
use armonika_core::ids::{PlaylistId, TrackId, UserId};
use armonika_core::model::{AudioFormat, FileFacts, TagFacts};
use armonika_core::play_queue;
use armonika_core::playlists::{self, PlaylistEdit};
use armonika_core::secret::SecretBox;
use armonika_core::users::{self, NewUser};
use armonika_core::{Error, libraries, track_files};

/// Users ann and bob who both see library A only, a library B nobody sees, and tracks
/// named after their file: A1 A2 A3 in A, h1 h2 in B (hidden from both).
struct World {
    _dir: tempfile::TempDir,
    db: Db,
    ann: Access,
    bob: Access,
    tracks: HashMap<&'static str, TrackId>,
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let music = tempfile::tempdir().unwrap();
    let music_b = tempfile::tempdir().unwrap();
    let db = Db::open(&dir.path().join("armonika.db")).unwrap();
    let sb = SecretBox::from_key([7; 32]);
    let (ann, bob, tracks) = db
        .write_blocking(|c| {
            let user =
                |c: &mut rusqlite::Connection, name: &str| -> armonika_core::Result<Access> {
                    let id: UserId = users::create(
                        c,
                        &sb,
                        NewUser {
                            name: name.to_owned(),
                            password: format!("{name}-password"),
                            is_admin: false,
                            can_download: false,
                        },
                    )?;
                    Ok(Access {
                        user: id,
                        name: name.to_owned(),
                        is_admin: false,
                        can_download: false,
                        libraries: Vec::new(),
                    })
                };
            let mut ann = user(c, "ann")?;
            let mut bob = user(c, "bob")?;
            let lib_a = libraries::create(c, "A", music.path(), dir.path())?;
            let lib_b = libraries::create(c, "B", music_b.path(), dir.path())?;
            for who in [&mut ann, &mut bob] {
                armonika_core::access::grant(c, &who.user, lib_a, Permission::Listen)?;
                who.libraries = vec![(lib_a, Permission::Listen)];
            }
            let facts = FileFacts {
                size: 1,
                mtime_ms: 1,
                format: AudioFormat::Flac,
                duration_ms: Some(1000),
                bitrate_kbps: None,
                sample_rate: None,
                channels: None,
                bit_depth: None,
                tags: TagFacts::default(),
            };
            let tx = c.transaction()?;
            let mut tracks = HashMap::new();
            for (name, lib) in [
                ("A1", lib_a),
                ("A2", lib_a),
                ("A3", lib_a),
                ("h1", lib_b),
                ("h2", lib_b),
            ] {
                let id = track_files::insert_track(&tx, lib, name, &facts, 1)?;
                tracks.insert(name, id);
            }
            tx.commit()?;
            Ok((ann, bob, tracks))
        })
        .unwrap();
    World {
        _dir: dir,
        db,
        ann,
        bob,
        tracks,
    }
}

impl World {
    fn ids(&self, names: &[&str]) -> Vec<TrackId> {
        names.iter().map(|n| self.tracks[n].clone()).collect()
    }

    fn create(&self, names: &[&str]) -> PlaylistId {
        let ids = self.ids(names);
        self.db
            .write_blocking(|c| playlists::create(c, &self.ann.user, "mix", &ids))
            .unwrap()
    }

    /// File names of every entry in stored order, hidden ones included.
    fn full(&self, pl: &PlaylistId) -> Vec<String> {
        self.db
            .read_blocking(|c| {
                Ok(c.prepare(
                    "SELECT t.rel_path FROM playlist_entries e
                     JOIN tracks t ON t.id = e.track_id
                     JOIN playlists p ON p.id = e.playlist_id
                     WHERE p.public_id = ?1 ORDER BY e.position",
                )?
                .query_map([pl.as_str()], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?)
            })
            .unwrap()
    }

    fn positions(&self, pl: &PlaylistId) -> Vec<i64> {
        self.db
            .read_blocking(|c| {
                Ok(c.prepare(
                    "SELECT e.position FROM playlist_entries e
                     JOIN playlists p ON p.id = e.playlist_id
                     WHERE p.public_id = ?1 ORDER BY e.position",
                )?
                .query_map([pl.as_str()], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?)
            })
            .unwrap()
    }

    fn visible(&self, viewer: &Access, pl: &PlaylistId) -> Option<Vec<String>> {
        self.db
            .read_blocking(|c| playlists::get(c, viewer, pl))
            .unwrap()
            .map(|(_, tracks)| tracks.into_iter().map(|t| t.path).collect())
    }

    fn replace(&self, pl: &PlaylistId, names: &[&str]) {
        let ids = self.ids(names);
        self.db
            .write_blocking(|c| playlists::replace_visible(c, &self.ann, pl, &ids))
            .unwrap();
    }

    fn remove(&self, pl: &PlaylistId, indexes: &[usize]) {
        self.db
            .write_blocking(|c| {
                playlists::update(
                    c,
                    &self.ann,
                    pl,
                    PlaylistEdit {
                        remove_visible_indexes: indexes.to_vec(),
                        ..PlaylistEdit::default()
                    },
                )
            })
            .unwrap();
    }

    fn purge_track(&self, rel_path: &'static str) {
        self.db
            .write_blocking(|c| {
                c.execute("DELETE FROM tracks WHERE rel_path = ?1", [rel_path])?;
                Ok(())
            })
            .unwrap();
    }
}

#[test]
fn public_playlist_is_read_only_for_others() {
    let w = world();
    let pl = w.create(&["A1", "A2"]);
    w.db.write_blocking(|c| {
        playlists::update(
            c,
            &w.ann,
            &pl,
            PlaylistEdit {
                public: Some(true),
                ..PlaylistEdit::default()
            },
        )
    })
    .unwrap();
    assert_eq!(w.visible(&w.bob, &pl).unwrap(), ["A1", "A2"]);
    let listed = w.db.read_blocking(|c| playlists::list(c, &w.bob)).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].owner_name, "ann");

    let edit = w.db.write_blocking(|c| {
        playlists::update(
            c,
            &w.bob,
            &pl,
            PlaylistEdit {
                name: Some("mine".to_owned()),
                ..PlaylistEdit::default()
            },
        )
    });
    assert!(matches!(edit, Err(Error::NotPlaylistOwner(_))), "{edit:?}");
    let replace =
        w.db.write_blocking(|c| playlists::replace_visible(c, &w.bob, &pl, &[]));
    assert!(matches!(replace, Err(Error::NotPlaylistOwner(_))));
    let delete = w.db.write_blocking(|c| playlists::delete(c, &w.bob, &pl));
    assert!(matches!(delete, Err(Error::NotPlaylistOwner(_))));
    assert_eq!(w.full(&pl), ["A1", "A2"]);
}

#[test]
fn private_playlist_invisible_to_others() {
    let w = world();
    let pl = w.create(&["A1"]);
    assert!(w.visible(&w.bob, &pl).is_none());
    assert!(
        w.db.read_blocking(|c| playlists::list(c, &w.bob))
            .unwrap()
            .is_empty()
    );
    let edit =
        w.db.write_blocking(|c| playlists::update(c, &w.bob, &pl, PlaylistEdit::default()));
    assert!(matches!(edit, Err(Error::PlaylistNotFound(_))), "{edit:?}");
    let delete = w.db.write_blocking(|c| playlists::delete(c, &w.bob, &pl));
    assert!(matches!(delete, Err(Error::PlaylistNotFound(_))));
    assert_eq!(w.visible(&w.ann, &pl).unwrap(), ["A1"]);
}

#[test]
fn same_track_twice_allowed() {
    let w = world();
    let pl = w.create(&["A1", "A1", "A2"]);
    assert_eq!(w.visible(&w.ann, &pl).unwrap(), ["A1", "A1", "A2"]);
    let (view, _) =
        w.db.read_blocking(|c| playlists::get(c, &w.ann, &pl))
            .unwrap()
            .unwrap();
    assert_eq!(view.song_count, 3);
}

#[test]
fn hidden_entries_not_returned_but_kept() {
    let w = world();
    let pl = w.create(&["A1", "h1", "A2"]);
    assert_eq!(w.visible(&w.ann, &pl).unwrap(), ["A1", "A2"]);
    let (view, _) =
        w.db.read_blocking(|c| playlists::get(c, &w.ann, &pl))
            .unwrap()
            .unwrap();
    assert_eq!(view.song_count, 2);
    assert_eq!(view.duration_ms, 2000);
    // Adding goes to the end of the full list.
    let add = w.ids(&["A3"]);
    w.db.write_blocking(|c| {
        playlists::update(
            c,
            &w.ann,
            &pl,
            PlaylistEdit {
                add,
                ..PlaylistEdit::default()
            },
        )
    })
    .unwrap();
    assert_eq!(w.full(&pl), ["A1", "h1", "A2", "A3"]);
}

#[test]
fn remove_by_visible_index_keeps_hidden() {
    let w = world();
    let pl = w.create(&["A1", "h1", "A2", "h2", "A3"]);
    w.remove(&pl, &[1]);
    assert_eq!(w.full(&pl), ["A1", "h1", "h2", "A3"]);
    // Out of range indexes change nothing.
    w.remove(&pl, &[7]);
    assert_eq!(w.full(&pl), ["A1", "h1", "h2", "A3"]);
}

#[test]
fn replace_keeps_hidden_after_anchor() {
    let w = world();
    let pl = w.create(&["A1", "h1", "A2", "A3"]);
    w.replace(&pl, &["A3", "A1", "A2"]);
    assert_eq!(w.full(&pl), ["A3", "A1", "h1", "A2"]);
}

#[test]
fn replace_moves_orphan_run_back() {
    let w = world();
    let pl = w.create(&["A1", "A2", "h1", "A3"]);
    w.replace(&pl, &["A1", "A3"]);
    assert_eq!(w.full(&pl), ["A1", "h1", "A3"]);
}

#[test]
fn replace_with_duplicates_matches_occurrences() {
    let w = world();
    let pl = w.create(&["A1", "h1", "A1", "h2"]);
    w.replace(&pl, &["A1", "A1"]);
    assert_eq!(w.full(&pl), ["A1", "h1", "A1", "h2"]);
}

#[test]
fn replace_keeps_the_leading_run_and_orphans_of_the_first_entry() {
    let w = world();
    let pl = w.create(&["h1", "A1", "h2", "A2"]);
    // A1 is dropped: its run moves to the start, after the leading run.
    w.replace(&pl, &["A2"]);
    assert_eq!(w.full(&pl), ["h1", "h2", "A2"]);
    // A new track in front keeps the leading run first.
    w.replace(&pl, &["A3", "A2"]);
    assert_eq!(w.full(&pl), ["h1", "h2", "A3", "A2"]);
}

#[test]
fn missing_track_hidden_then_visible_again() {
    let w = world();
    let pl = w.create(&["A1", "A2", "A3"]);
    let a2 = w.tracks["A2"].clone();
    w.db.write_blocking(|c| {
        let tx = c.transaction()?;
        track_files::mark_missing(&tx, std::slice::from_ref(&a2))?;
        Ok(tx.commit()?)
    })
    .unwrap();
    assert_eq!(w.visible(&w.ann, &pl).unwrap(), ["A1", "A3"]);
    // Index 1 of the visible list is A3: the hidden A2 stays.
    w.remove(&pl, &[1]);
    assert_eq!(w.full(&pl), ["A1", "A2"]);
    w.db.write_blocking(|c| {
        let tx = c.transaction()?;
        track_files::set_present_at(&tx, &a2, "A2")?;
        Ok(tx.commit()?)
    })
    .unwrap();
    assert_eq!(w.visible(&w.ann, &pl).unwrap(), ["A1", "A2"]);
}

#[test]
fn gaps_left_by_a_purge_are_tolerated_and_closed() {
    let w = world();
    let pl = w.create(&["A1", "A2", "h1", "A3"]);
    // A purge deletes the track row: the entry goes by cascade and leaves a gap.
    w.purge_track("A2");
    assert_eq!(w.positions(&pl), [0, 2, 3]);
    assert_eq!(w.visible(&w.ann, &pl).unwrap(), ["A1", "A3"]);
    w.remove(&pl, &[1]);
    assert_eq!(w.full(&pl), ["A1", "h1"]);
    assert_eq!(w.positions(&pl), [0, 1]);

    let pl = w.create(&["A1", "A3", "h1"]);
    w.purge_track("A1");
    assert_eq!(w.positions(&pl), [1, 2]);
    w.replace(&pl, &["A3", "A3"]);
    assert_eq!(w.full(&pl), ["A3", "h1", "A3"]);
    assert_eq!(w.positions(&pl), [0, 1, 2]);
}

#[test]
fn update_changes_name_comment_and_public_and_delete_removes() {
    let w = world();
    let pl = w.create(&["A1"]);
    w.db.write_blocking(|c| {
        playlists::update(
            c,
            &w.ann,
            &pl,
            PlaylistEdit {
                name: Some("renamed".to_owned()),
                comment: Some("note".to_owned()),
                public: Some(true),
                ..PlaylistEdit::default()
            },
        )
    })
    .unwrap();
    let (view, _) =
        w.db.read_blocking(|c| playlists::get(c, &w.ann, &pl))
            .unwrap()
            .unwrap();
    assert_eq!(view.name, "renamed");
    assert_eq!(view.comment.as_deref(), Some("note"));
    assert!(view.public);
    w.db.write_blocking(|c| playlists::delete(c, &w.ann, &pl))
        .unwrap();
    assert!(w.visible(&w.ann, &pl).is_none());
}

fn save_queue(w: &World, names: &[&str], current: Option<usize>, ms: i64, client: &str) {
    let ids = w.ids(names);
    w.db.write_blocking(|c| play_queue::save(c, &w.ann.user, &ids, current, ms, client))
        .unwrap();
}

fn load_queue(w: &World, viewer: &Access) -> Option<play_queue::PlayQueueView> {
    w.db.read_blocking(|c| play_queue::load(c, viewer)).unwrap()
}

fn queue_paths(q: &play_queue::PlayQueueView) -> Vec<&str> {
    q.tracks.iter().map(|t| t.path.as_str()).collect()
}

#[test]
fn play_queue_last_save_wins() {
    let w = world();
    assert!(load_queue(&w, &w.ann).is_none());
    save_queue(&w, &["A1", "A2"], Some(1), 500, "phone");
    save_queue(&w, &["A3", "A1"], Some(0), 1500, "desktop");
    let q = load_queue(&w, &w.ann).unwrap();
    assert_eq!(queue_paths(&q), ["A3", "A1"]);
    assert_eq!(q.current, Some(0));
    assert_eq!(q.position_ms, 1500);
    assert_eq!(q.changed_by, "desktop");
    // Another user has their own queue.
    assert!(load_queue(&w, &w.bob).is_none());
    // Saving an empty list clears it.
    save_queue(&w, &[], None, 0, "desktop");
    assert!(load_queue(&w, &w.ann).is_none());
}

#[test]
fn play_queue_hides_invisible_and_recomputes_current() {
    let w = world();
    save_queue(&w, &["h1", "A1", "h2", "A2", "A3"], Some(3), 42, "phone");
    let q = load_queue(&w, &w.ann).unwrap();
    assert_eq!(queue_paths(&q), ["A1", "A2", "A3"]);
    assert_eq!(q.current, Some(1));
    // The current entry itself is hidden: no current track.
    save_queue(&w, &["A1", "h1", "A2"], Some(1), 42, "phone");
    let q = load_queue(&w, &w.ann).unwrap();
    assert_eq!(q.current, None);
}

#[test]
fn play_queue_tolerates_gaps_from_a_purge() {
    let w = world();
    save_queue(&w, &["A1", "A2", "A3"], Some(2), 7, "phone");
    w.purge_track("A2");
    let q = load_queue(&w, &w.ann).unwrap();
    assert_eq!(queue_paths(&q), ["A1", "A3"]);
    assert_eq!(q.current, Some(1));
}
