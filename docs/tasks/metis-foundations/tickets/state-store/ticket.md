---
type: ticket
subtype: feature
title: Daemon state store on SQLite
status: stable
workflow_state: ready
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
