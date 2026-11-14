-- A track whose content hash was computed at least once. The hasher never merges such a
-- track into another one: only a file just found by the scan can be relinked to a
-- missing track (spec 8.7 point 1), and merging duplicates is a manual choice.
ALTER TABLE tracks ADD COLUMN hashed_once INTEGER NOT NULL DEFAULT 0;
UPDATE tracks SET hashed_once = 1 WHERE content_hash IS NOT NULL;
