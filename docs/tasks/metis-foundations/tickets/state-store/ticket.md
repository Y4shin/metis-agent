---
type: ticket
subtype: feature
title: Daemon state store on SQLite
status: deprecated
workflow_state: done
blocked_by: []
size: m
---

## What to build

The daemon's persistent state. One SQLite database opened through sqlx, opened
and migrated at startup, with the table the token hashes live in, so pairing and
the later session and message work share one store instead of inventing a file
format each.

`crates/daemon` gains a library target beside its binary, because
`docs/rough-plan.md` starts storage as an internal module of the daemon crate
rather than as a new crate or a module of `api-contracts`.

## Acceptance criteria

- [ ] The database is opened at the configured path, created when missing, with
      its parent directory created at mode 700 and the file at mode 600, asserted
      by a test that reads the resulting permissions.
- [ ] A committed migration creates the token table and runs on open, so a fresh
      database is usable without a separate migration step, asserted by a test on
      a new file.
- [ ] A token row stores the surface name and the SHA-256 hash only, never the
      token, and insert, verify and revoke round-trip, asserted by a test.
- [ ] Revoking one surface leaves the other surfaces verifying, asserted by a
      test with two rows.
- [ ] An insert for a surface that already has a row is refused with a typed
      error rather than replacing the row, and verification never returns a
      stored hash to its caller.
- [ ] CI needs no database server and no `DATABASE_URL`: the store uses sqlx
      runtime queries, not the compile-time macros, and no `.sqlx` offline data is
      committed.

## Blocked by

Nothing. This is the foundation `pairing-and-tokens` and the daemon effort build
on.

## Implementation notes

Landed on `task/state-store` as the merge commit `925371a`
("ticket(state-store): Daemon state store on SQLite"), parents `e88b055` (the
landing branch, created at this ticket's arch spec commit, which is also
`main`'s tip and carries every earlier landing merge) and `f8331b5` (working
branch `ticket/state-store`, eleven `wip:` commits). The merge was clean with no
conflicts, the landing tree is byte-identical to `f8331b5` except for this note
and the committed deviation report, and `ticket/state-store` was deleted after
the merge.

Gates, run on `f8331b5` with `DATABASE_URL` explicitly unset, through `devenv
shell --`: `cargo fmt --check` clean, `cargo clippy --all-targets -- -D
warnings` clean, `cargo test -p daemon` exit 0 with 10 passed and 0 failed,
`cargo test -p api-contracts` exit 0 with 23 passed and 0 failed, and the whole
workspace `cargo test` exit 0 with every target ok and 0 failed (99 passed, up
from 88). The merge is byte-identical, so no gate was re-run on the landing tree.

What landed: `crates/daemon` gained a library target beside its binary;
`src/state.rs` holds `STATE_DB_ENV`, `resolve_db_path`, `StateError`, `Verify`
and `Store::{open, open_in_memory, insert_token, verify_token, revoke_token,
surfaces}`; `src/token.rs` holds `mint`, `Token`, `TokenHash` and `TokenError`;
`migrations/0001_tokens.sql` creates `tokens(surface TEXT PRIMARY KEY, hash BLOB
NOT NULL, paired_at TEXT NOT NULL)` and is applied by `sqlx::migrate!` on open.
The path rule is the variable, then `$XDG_DATA_HOME/metis/metis.db`, then
`$HOME/.local/share/metis/metis.db`, then `StateError::NoStateDir`, with an
absent or empty variable treated as unset, as `METIS_ENDPOINT` and
`METIS_TCP_ADDR` already behave.

Divergences from the arch spec, additive and inside the recorded approval:

- `Verify` has `Verified`, `UnknownSurface` and `Mismatch` and is
  non-exhaustive. `pairing-and-tokens` tells a revoked or never-paired surface
  from a bad token without a second query, and no returned value can carry a
  hash.
- `insert_token`, `verify_token` and `revoke_token` take `surface: &str`, and
  `surfaces()` returns `Vec<String>`, so the store stays decoupled from
  `api_contracts::identity::Surface`. Every store method is `async` and returns
  `Result<_, StateError>`; migration failures reuse `StateError::Database`
  through `sqlx::Error::Migrate`.
- The hashed input is pinned: `Token::hash()` is the SHA-256 of the token's
  canonical lowercase hex **text**, which is what a client presents, not of the
  32 decoded bytes, and `Token::parse` normalises case. Known answer:
  `0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef` hashes to
  `a8ae6e6ee929abea3afcfc5258c8ccd6f85273e0d4626d26c7279f3250f77c8e`. This is a
  cross-ticket rule: `pairing-and-tokens` must hash the presented text exactly
  this way, or pairing appears to succeed while every later call is refused.
- `api_contracts::transport` gained `ensure_private_dir(dir, mode)` and
  `restrict_file(path, mode)`, both `io::Result<()>`; the private `SocketAcl`
  type became `PathAcl`. `ensure_socket_dir` and `serve_uds` keep their
  signatures and behaviour and `ConfigError::SocketAcl` keeps its fields, so the
  landing is additive for `windows-named-pipe` and `reverse-proxy-mode`. Only
  the refusal message wording changed.
- `sqlx` 0.9.0 (`default-features = false`; `runtime-tokio`, `sqlite`, `macros`,
  `migrate`), `getrandom` 0.4, `sha2` 0.11 and `subtle` 2 were added, with
  `Cargo.lock` growing by 921 lines. `libsqlite3-sys` now bundles a C SQLite
  library into every build.

Criterion 6 holds as stated: the daemon uses `sqlx::query`, `query_scalar` and
`query_as` at runtime, no `query!`, `query_as!` or `query_scalar!` macro appears
in `crates/daemon/src` or `crates/daemon/tests`, no `.sqlx` directory exists, and
`git diff main -- .github contracts buf.yaml buf.gen.yaml` is empty, so CI gained
no step and needs no database server.

Residual risks carried into `pairing-and-tokens` and the daemon effort:

- `Store::open` narrows a pre-existing widened database file to 600 instead of
  refusing it; the symlink, owner and wider-than-mode refusal rule applies to the
  directory, as it does for the socket directory.
- No rotation, pruning or stale-database unlink, and no test opens a database
  written by another version.
- No test asserts constant-time comparison, only that comparison works; the
  property rests on `subtle`'s `ct_eq`.
- The in-memory store is capped at one connection, because an in-memory SQLite
  database is per connection; file-backed stores use the default pool sizing.
- Non-Unix creation carries no mode rule, which `windows-named-pipe` owns.
- `docs/dev-env.md` (the variable, the default path, the modes) and
  `docs/testing.md` (the run commands and the no-server, no-`DATABASE_URL`
  property) were updated with the ticket, so finalize needs no further docs
  change for the store itself.

Knowledge harvested at finalize: the effort map gained the decision bullet for
the landed store, and the arch spec carries the cross-ticket hash rule that
`pairing-and-tokens` must match (SHA-256 over the token's canonical lowercase hex
text, with the known-answer digest). The deviation report's checkpoint count was
corrected from twelve to eleven, ten `wip:` commits plus one docs commit, to
match the landed history.
