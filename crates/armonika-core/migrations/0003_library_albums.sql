-- Library albums (spec 8.1 "Identita degli album della libreria", 8.7 point 10).
-- Album dates, computed by `reconcile_albums` from the present tracks of the album:
-- the most frequent release date and first release date.
ALTER TABLE library_albums ADD COLUMN date_y INTEGER;
ALTER TABLE library_albums ADD COLUMN date_m INTEGER;
ALTER TABLE library_albums ADD COLUMN date_d INTEGER;
ALTER TABLE library_albums ADD COLUMN orig_date_y INTEGER;
ALTER TABLE library_albums ADD COLUMN orig_date_m INTEGER;
ALTER TABLE library_albums ADD COLUMN orig_date_d INTEGER;
-- Copies of tags that `reconcile_albums` needs to rebuild album credits and keys from
-- the database alone. The multi-value tags (ARTISTS, ALBUMARTISTS and the MusicBrainz
-- artist ids) hold their values joined by U+001F; NULL when the tag is empty.
ALTER TABLE tracks ADD COLUMN artists_tag TEXT;
ALTER TABLE tracks ADD COLUMN album_artists_tag TEXT;
ALTER TABLE tracks ADD COLUMN mb_artist_ids TEXT;
ALTER TABLE tracks ADD COLUMN mb_album_artist_ids TEXT;
-- The album title as `text::normalize` folds it, to find the tracks of an album title
-- without reading the whole library.
ALTER TABLE tracks ADD COLUMN album_title_norm TEXT;
CREATE INDEX tracks_album_title ON tracks(library_id, album_title_norm) WHERE state = 'present';
-- Tracks read before this migration have none of these columns: the next scan reads
-- their tags again (a tag reader version older than any real one).
UPDATE tracks SET tag_schema = 0;
