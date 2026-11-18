-- ISRCs read from the tags (values joined by U+001F) and the major version of the ID3v2
-- header of mp3 files. TAG_SCHEMA_VERSION 2 makes the scanner fill them on every track.
ALTER TABLE tracks ADD COLUMN isrcs TEXT;
ALTER TABLE tracks ADD COLUMN id3v2_major INTEGER;
