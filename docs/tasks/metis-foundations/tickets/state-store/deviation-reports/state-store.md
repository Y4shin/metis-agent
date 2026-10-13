---
type: deviation report
title: Deviation report for state-store
status: stable
---

## Deviation report: state-store

Implemented on `ticket/state-store` (HEAD `f8331b5`, twelve `wip:` checkpoints
off `main` at `e88b055`). Compared against the `## Pending ticket: state-store`
section of `docs/tasks/metis-foundations/arch-spec.md` and the ticket doc's
acceptance criteria. I re-ran the gates on `f8331b5`: `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test -p daemon -p
api-contracts`, and the whole workspace `cargo test` (99 passed, 0 failed, up
from 88), with `DATABASE_URL` explicitly unset. All green.

### API surface changes

- **Planned:** the spec named `Verify` as the return of `verify_token` without
  naming its variants.
  **Actual:** `Verify::{Verified, UnknownSurface, Mismatch}`, non-exhaustive. The
  three are distinguishable on purpose, so a caller can tell a revoked or
  never-paired surface from a bad token. The enum carries no hash, so a caller
  that verifies never holds a value it could replay, which is how criterion 5's
  "verification never returns a stored hash" holds structurally rather than by
  test.
  **Impact:** `pairing-and-tokens` maps `UnknownSurface` and `Mismatch` onto its
  two rejection causes. It is the change that makes the ticket's "revoked versus
  invalid" distinction expressible without a second query.

- **Planned:** `insert_token(&self, surface, &TokenHash)`, `verify_token`, and
  `revoke_token` with the parameter and return types unstated.
  **Actual:** `surface: &str`, and `surfaces() -> Result<Vec<String>,
  StateError>`. The store stays decoupled from `api_contracts::identity::Surface`
  and from the identity module entirely.
  **Impact:** the pairing ticket passes the surface name it already reads from
  the declaration headers. No type conversion layer is needed between the two
  crates, and the daemon crate does not depend on the identity types.

- **Planned:** `Token::parse(&str)`, with no error type named.
  **Actual:** `Token::parse(&str) -> Result<Token, TokenError>` with the new
  `TokenError::Malformed`, non-exhaustive and implementing `Display` plus
  `Error`.
  **Impact:** a malformed credential from the wire becomes a typed refusal
  rather than a panic, which is what the pairing ticket needs to answer
  `Unauthenticated` instead of ending the connection.

- **Planned:** "`TokenHash` is its SHA-256", with the hashed input unspecified.
  **Actual:** the digest is taken over the token's canonical lowercase hex
  **text**, not over the 32 decoded bytes, and `Token::parse` normalises case so
  the digest does not depend on how a client wrote the text. A known-answer test
  pins it: the digest of
  `0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef` is
  `a8ae6e6ee929abea3afcfc5258c8ccd6f85273e0d4626d26c7279f3250f77c8e`.
  **Impact:** this is the one wire-visible rule the ticket fixes, and
  `pairing-and-tokens` must hash the presented text exactly this way. A client
  that hashes decoded bytes produces a credential that never verifies, and no
  test outside this ticket would catch it. Worth naming in the ticket's
  implementation note and in the pairing section.

- **Planned:** no return types on the store methods, and `Store::open(path)`
  without a signature detail.
  **Actual:** every store method is `async` and returns `Result<_, StateError>`,
  and `Store::open`/`open_in_memory` are `async` too. Migration failures reuse
  `StateError::Database` through `sqlx::Error::Migrate`, so no variant beyond the
  spec's four was needed.
  **Impact:** callers are already async (the daemon and the tests), so nothing
  else moves.

- **Planned:** the landed `ensure_socket_dir` "generalises into a private
  directory helper that takes the mode", with no signature the caller sees today
  changing.
  **Actual:** two new public functions in `api-contracts::transport`,
  `ensure_private_dir(dir, mode)` and `restrict_file(path, mode)`, both
  `io::Result<()>`. The private `SocketAcl` type became `PathAcl` with a
  `private(mode)` constructor. `ensure_socket_dir` and `serve_uds` keep their
  signatures and behaviour, `ConfigError::SocketAcl` keeps its fields, and the
  `path` in a directory refusal still names the directory. The internal refusal
  messages were reworded to name the path they refuse.
  **Impact:** additive for every dependent ticket. `windows-named-pipe` and
  `reverse-proxy-mode` get the two mode-taking helpers; the landed ACL tests of
  `local-identity-and-acl` pass unchanged, which the `api-contracts` run above
  confirms. The rewording is visible only through an error's message text, which
  no test asserts.

- **Planned:** the spec named the sqlx feature set, not versions.
  **Actual:** `sqlx` 0.9.0 (`default-features = false`, `runtime-tokio`,
  `sqlite`, `macros`, `migrate`), `getrandom` 0.4, `sha2` 0.11, `subtle` 2, with
  `Cargo.lock` growing by 921 lines.
  **Impact:** the bundled `libsqlite3-sys` C library is now part of every build,
  as the spec's recorded residual says, and `sqlx` 0.9 needs a recent toolchain
  (its MSRV is 1.94; the devenv shell is on 1.101). No other crate depends on
  these yet, so `pairing-and-tokens` is the first consumer.

- **Planned:** the state directory at 700 and the file at 600.
  **Actual:** as planned on Unix, and `prepare` creates both without a mode on
  non-Unix, with a comment that the front-end for that platform brings its own
  rule.
  **Impact:** matches the spec's Unix-gated creation seam and leaves the Windows
  rule to `windows-named-pipe`. The creation seam's assertions are Unix-gated
  the way the landed ACL tests are.

- **Planned:** "the variable wins, then `$XDG_DATA_HOME/metis/metis.db`, then
  `$HOME/.local/share/metis/metis.db`, then `StateError::NoStateDir`".
  **Actual:** exactly that, with an absent **or empty** variable treated as
  unset, so `METIS_STATE_DB=""` falls back to the data home rather than naming
  the current directory.
  **Impact:** consistent with the landed `METIS_ENDPOINT` and `METIS_TCP_ADDR`
  rules. `pairing-and-tokens` and the daemon effort inherit the same three-step
  fallback.

### Abstraction usage

- Used/was specified: yes.
  - The landed private-path rule is reused, not copied: the store calls
    `ensure_private_dir` and `restrict_file`, which is the same code path
    `serve_uds` uses for the socket directory and the socket file.
  - `ConfigError`'s non-exhaustive convention carried over to `StateError`,
    `TokenError` and `Verify`.
  - The RAII environment helper pattern from `crates/client/tests/{echo,acl}.rs`
    is reproduced inside the daemon's `tests/path.rs` (`StateEnv`, with its own
    lock and restore-on-drop), because the client crate's helper is not visible
    across crates. Same pattern, no new mechanism.
  - `sqlx` runtime queries only: zero occurrences of `query!`, `query_as!` or
    `query_scalar!` in `crates/daemon/src` and `crates/daemon/tests`, and no
    `.sqlx` directory in the tree. Verified by grep and by `ls`.
  - The migration is committed SQL (`migrations/0001_tokens.sql`) applied by
    `sqlx::migrate!`, so the binary embeds its schema and CI gained no step:
    `git diff main...HEAD -- .github contracts buf.yaml buf.gen.yaml scripts` is
    empty.
  - `docs/dev-env.md` and `docs/testing.md` were updated as the spec's exports
    list required, including the no-server, no-`DATABASE_URL` property.

### Out-of-scope changes

- `crates/api-contracts/src/transport.rs`: besides the two new helpers, the
  private `SocketAcl` type was renamed to `PathAcl`, and the refusal messages
  were reworded from "the socket directory is ..." to "cannot use <path>: it
  is ...". Internal and cosmetic, no variant or field changed.
- `crates/daemon/Cargo.toml` gained `[lib]` and moved `sqlx` into
  `[dev-dependencies]` as well, because the migration seam reads the file the
  store created with an independent connection.
- `Cargo.lock`: 921 lines, all dependency resolution.
- Nothing was removed. No generated tree, contract, CI workflow, or plugin ABI
  surface changed.

### Ticket doc update needed?

Yes. The ticket has no `## Implementation notes` section (it ends at
`## Blocked by`), so the land-worker must create the heading and record:

- the landing paragraph (merge commit, landing branch, working branch), as the
  three earlier tickets' notes do;
- the `Verify` variants and the `&str` store signatures, with the reason the
  store stays decoupled from `identity::Surface`;
- the pinned hash input rule and the known-answer digest, flagged as the rule
  `pairing-and-tokens` must match;
- the two new `api-contracts` helpers and the `SocketAcl` to `PathAcl` rename,
  with the note that the landed signatures and `ConfigError::SocketAcl` fields
  are unchanged;
- the dependency additions with versions, and the bundled `libsqlite3-sys` C
  library;
- the gates re-run here (fmt, clippy, `-p daemon -p api-contracts`, the whole
  workspace at 99 passed and 0 failed) with `DATABASE_URL` unset;
- the residuals: `Store::open` narrows a pre-existing widened file to 600
  instead of refusing it, while the directory keeps the symlink/owner/wider
  refusal; no rotation, pruning or stale-database unlink; the in-memory pool is
  capped at one connection because an in-memory SQLite database is per
  connection; non-Unix creation carries no mode rule; no test asserts
  constant-time behaviour, only that comparison works, and no test opens a
  database written by another version.

### User attention needed?

No, and no decision is open. The divergences are additive and inside the
approved spec, and the recorded approval (`state-store/.work/approval.md`)
already covers the dependencies, the path rule and the migration mechanism.

Two facts are worth stating plainly rather than asking about:

- The hash-input rule is now a cross-ticket contract with a known-answer test.
  If `pairing-and-tokens` hashes anything other than the token's canonical
  lowercase hex text, pairing will appear to succeed and every later call will be
  refused, which the pairing ticket's own tests would show as a mystery rather
  than as this rule.
- The ticket adds three dependencies and a bundled C SQLite library to the
  workspace, which the operator approved as the state-store decision (ADR 0003).
  Nothing in the plugin, contract, or client builds changed behaviour, and the
  full workspace suite is green.
