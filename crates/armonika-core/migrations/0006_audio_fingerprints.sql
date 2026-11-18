-- Chromaprint fingerprints of the audio, keyed by the content hash so that copies of the
-- same audio share one. A failure is kept too, so that a file that cannot be
-- fingerprinted is not tried again on every pass.
CREATE TABLE audio_fingerprints (
  content_hash BLOB PRIMARY KEY,
  fingerprint TEXT NOT NULL,
  decoder TEXT NOT NULL,
  computed_at INTEGER NOT NULL
);

CREATE TABLE audio_fingerprint_failures (
  content_hash BLOB PRIMARY KEY,
  reason TEXT NOT NULL,
  failed_at INTEGER NOT NULL
);
