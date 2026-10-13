# 0003. The daemon's state lives in one SQLite database through sqlx

- Status: accepted
- Date: 2026-10-13

## Context

The daemon needs persistent state: token hashes now, and sessions, messages and
workflow state as later efforts land. The alternatives were a file per concern
(a text file for token hashes, another for sessions), an embedded key-value
store, or a relational database. A file per concern was the shape the pairing
ticket originally planned for token hashes alone, and it does not extend to
session history, message ordering or the queries that go with them.

The decision: one SQLite database, opened through `sqlx` with the `sqlite`
feature and runtime queries, living at `METIS_STATE_DB`, defaulting to
`$XDG_DATA_HOME/metis/metis.db` and `~/.local/share/metis/metis.db` when that
variable is unset. The file is mode 600 and its directory mode 700. Schema
changes go through committed migrations under the daemon crate, applied on open,
and the store is an internal module of `crates/daemon` rather than a new crate,
following `docs/rough-plan.md`'s instruction to start storage as an internal
module and extract it only when reuse or complexity justifies that.

Runtime queries rather than `sqlx`'s compile-time macros keep CI free of a
database server, a `DATABASE_URL` and committed `.sqlx` offline data.

## Consequences

- One file backs up or moves all daemon state, and revocation is a `DELETE`.
- The build gains a C dependency (`libsqlite3-sys`, bundled), and `sqlx` brings
  its own async machinery. That is the cost of the relational features the later
  session and message work needs.
- Secrets stay out of the database by construction: token rows hold a SHA-256
  hash, and the preshared key is never stored at all.
- The store API is the seam the pairing service and the later daemon work call,
  so schema details stay inside the daemon crate instead of spreading into the
  wire contract crate.
