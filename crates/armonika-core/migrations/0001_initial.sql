-- Phase 1 schema. Times are Unix milliseconds (INTEGER).
-- Convention (review against the MusicBrainz schema, point 8): every entity has an
-- internal key `id INTEGER PRIMARY KEY` used by all links and a stable
-- `public_id TEXT UNIQUE`, the only one exposed in the APIs.
CREATE TABLE users (
  id INTEGER PRIMARY KEY, public_id TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL UNIQUE COLLATE NOCASE,
  password_hash TEXT NOT NULL,              -- argon2id, PHC format
  subsonic_password BLOB NOT NULL,          -- SecretBox
  is_admin INTEGER NOT NULL DEFAULT 0, can_download INTEGER NOT NULL DEFAULT 1,
  share_now_playing INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL);
CREATE TABLE api_keys (
  key_hash TEXT PRIMARY KEY,                -- hex blake3 of the key
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  label TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE TABLE libraries (
  id INTEGER PRIMARY KEY,                   -- exposed as is: apps want an integer
  name TEXT NOT NULL UNIQUE, root TEXT NOT NULL,
  write_level TEXT NOT NULL DEFAULT 'read_only'
    CHECK (write_level IN ('read_only','tags','tags_and_organize')),
  created_at INTEGER NOT NULL);
CREATE TABLE library_access (
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  library_id INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
  permission TEXT NOT NULL CHECK (permission IN ('listen','manage')),
  PRIMARY KEY (user_id, library_id));
CREATE TABLE artists (
  id INTEGER PRIMARY KEY, public_id TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL, norm_name TEXT NOT NULL, sort_name TEXT, mbid TEXT UNIQUE,
  updated_at INTEGER NOT NULL);
CREATE UNIQUE INDEX artists_name_only ON artists(norm_name) WHERE mbid IS NULL;
CREATE TABLE library_albums (
  id INTEGER PRIMARY KEY, public_id TEXT NOT NULL UNIQUE,
  library_id INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
  group_key TEXT NOT NULL, title TEXT NOT NULL, sort_title TEXT, artist_credit TEXT NOT NULL,
  release_mbid TEXT, release_group_mbid TEXT, is_singles INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE INDEX library_albums_key ON library_albums(library_id, group_key);
CREATE TABLE album_artists (
  album_id INTEGER NOT NULL REFERENCES library_albums(id) ON DELETE CASCADE,
  position INTEGER NOT NULL, artist_id INTEGER NOT NULL REFERENCES artists(id),
  credited_name TEXT NOT NULL, join_phrase TEXT NOT NULL DEFAULT '',
  PRIMARY KEY (album_id, position));
CREATE TABLE tracks (
  id INTEGER PRIMARY KEY, public_id TEXT NOT NULL UNIQUE,
  library_id INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
  state TEXT NOT NULL CHECK (state IN ('present','missing','deleted')),
  rel_path TEXT NOT NULL, size INTEGER NOT NULL, mtime_ms INTEGER NOT NULL,
  tag_schema INTEGER NOT NULL,              -- TAG_SCHEMA_VERSION the tags were read with
  format TEXT NOT NULL, duration_ms INTEGER, bitrate_kbps INTEGER, sample_rate INTEGER,
  channels INTEGER, bit_depth INTEGER,
  content_hash BLOB, hash_state TEXT NOT NULL DEFAULT 'pending'
    CHECK (hash_state IN ('pending','done','failed')),
  prev_content_hash BLOB,                   -- hash before a pre-filter relink
  prev_rel_path TEXT,                       -- path before a pre-filter relink
  title TEXT, sort_title TEXT, artist_credit TEXT, artist_sort TEXT,
  album_title TEXT, album_sort TEXT, album_artist_credit TEXT, album_artist_sort TEXT,
  track_no INTEGER,                         -- for sorting; NULL if the tag is not a number
  track_number_text TEXT,                   -- original tag value (e.g. "A1" on vinyl)
  track_total INTEGER, disc_no INTEGER, disc_total INTEGER, disc_subtitle TEXT,
  date_y INTEGER, date_m INTEGER, date_d INTEGER,                 -- release date
  orig_date_y INTEGER, orig_date_m INTEGER, orig_date_d INTEGER,  -- first release
  compilation INTEGER NOT NULL DEFAULT 0, has_embedded_cover INTEGER NOT NULL DEFAULT 0,
  mb_recording_id TEXT, mb_release_track_id TEXT, mb_release_id TEXT,
  mb_release_group_id TEXT, mb_work_id TEXT,
  library_album_id INTEGER REFERENCES library_albums(id),
  added_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, missing_since INTEGER);
CREATE UNIQUE INDEX tracks_present_path ON tracks(library_id, rel_path) WHERE state = 'present';
CREATE INDEX tracks_album ON tracks(library_album_id);
CREATE INDEX tracks_prefilter ON tracks(library_id, size, mtime_ms) WHERE state = 'missing';
CREATE INDEX tracks_hash ON tracks(library_id, content_hash);
CREATE INDEX tracks_hash_pending ON tracks(hash_state) WHERE hash_state = 'pending';
CREATE TABLE track_artists (
  track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
  position INTEGER NOT NULL, artist_id INTEGER NOT NULL REFERENCES artists(id),
  credited_name TEXT NOT NULL, join_phrase TEXT NOT NULL DEFAULT '',
  PRIMARY KEY (track_id, position));
CREATE TABLE track_genres (
  track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE, genre TEXT NOT NULL,
  PRIMARY KEY (track_id, genre));
CREATE TABLE id_redirects (                 -- like the MusicBrainz *_gid_redirect tables
  old_public_id TEXT PRIMARY KEY, new_public_id TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE TABLE track_annotations (
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
  starred_at INTEGER, rating INTEGER CHECK (rating BETWEEN 1 AND 5),
  play_count INTEGER NOT NULL DEFAULT 0, last_played_at INTEGER,
  PRIMARY KEY (user_id, track_id));
CREATE TABLE album_annotations (
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  album_id INTEGER NOT NULL REFERENCES library_albums(id) ON DELETE CASCADE,
  starred_at INTEGER, rating INTEGER CHECK (rating BETWEEN 1 AND 5),
  PRIMARY KEY (user_id, album_id));
CREATE TABLE artist_annotations (
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  artist_id INTEGER NOT NULL REFERENCES artists(id) ON DELETE CASCADE,
  starred_at INTEGER, rating INTEGER CHECK (rating BETWEEN 1 AND 5),
  PRIMARY KEY (user_id, artist_id));
CREATE TABLE play_history (
  id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE, played_at INTEGER NOT NULL);
CREATE TABLE playlists (
  id INTEGER PRIMARY KEY, public_id TEXT NOT NULL UNIQUE,
  owner_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name TEXT NOT NULL, comment TEXT, public INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE playlist_entries (
  playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
  position INTEGER NOT NULL, track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
  PRIMARY KEY (playlist_id, position));
CREATE TABLE play_queues (
  user_id INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  current_index INTEGER, position_ms INTEGER NOT NULL DEFAULT 0,
  changed_at INTEGER NOT NULL, changed_by TEXT NOT NULL);
CREATE TABLE play_queue_entries (
  user_id INTEGER NOT NULL REFERENCES play_queues(user_id) ON DELETE CASCADE,
  position INTEGER NOT NULL, track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
  PRIMARY KEY (user_id, position));
CREATE TABLE now_playing (
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE, client TEXT NOT NULL,
  track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE, started_at INTEGER NOT NULL,
  PRIMARY KEY (user_id, client));
CREATE TABLE scrobble_accounts (
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  service TEXT NOT NULL CHECK (service IN ('lastfm','listenbrainz')),
  secret BLOB NOT NULL, account_name TEXT, PRIMARY KEY (user_id, service));
CREATE TABLE scrobble_outbox (
  id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  service TEXT NOT NULL, track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
  played_at INTEGER NOT NULL, attempts INTEGER NOT NULL DEFAULT 0, next_attempt_at INTEGER NOT NULL);
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
