-- Artist links independent of scan order (spec 8.7 point 7).
-- matched_by_name = 1: the credit had no MusicBrainz id and was attached to an artist
-- with one because that artist is the only one with this normalized name in the
-- library. `reconcile_artists` re-checks these rows (and the rows of name-only
-- artists) at the end of each scan.
ALTER TABLE track_artists ADD COLUMN matched_by_name INTEGER NOT NULL DEFAULT 0;
ALTER TABLE album_artists ADD COLUMN matched_by_name INTEGER NOT NULL DEFAULT 0;
-- Lookup of artists with an mbid by normalized name (the name-only index covers the rest).
CREATE INDEX artists_norm_mbid ON artists(norm_name) WHERE mbid IS NOT NULL;
