-- The per-surface credentials: the surface a token was issued to, the hash of
-- that token, and when it was paired.
--
-- The token itself is never written here, so a copy of this file cannot be
-- replayed against the daemon. `hash` is the SHA-256 of the token's canonical
-- lowercase hex text, which is the form the token travels in.
CREATE TABLE tokens (
    surface   TEXT PRIMARY KEY,
    hash      BLOB NOT NULL,
    paired_at TEXT NOT NULL
);
