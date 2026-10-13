# Approval: `state-store` architecture spec

- Approved: 2026-10-13, in chat, by the operator.
- Approved artifact: `docs/tasks/metis-foundations/arch-spec.md`, section
  `## Pending ticket: state-store`.
- Ticket: `docs/tasks/metis-foundations/tickets/state-store/ticket.md`.

## Why this ticket exists

The operator's decision on 2026-10-13 redirected `pairing-and-tokens`: the
daemon stores its state in SQLite through sqlx rather than in a file per
concern. That is more than the pairing work alone was sized for, so the store
was split into this ticket and `pairing-and-tokens` now lists `state-store` in
its `blocked_by`.

Placement follows `docs/rough-plan.md` line 191: storage starts as an internal
module of `crates/daemon`, and a crate is extracted only when reuse or
complexity justifies that. So the daemon crate gains a library target beside its
binary, and the store lives at `crates/daemon/src/state.rs` with token minting
and hashing at `crates/daemon/src/token.rs`.

## Decisions confirmed at approval

- `METIS_STATE_DB` first, then `$XDG_DATA_HOME/metis/metis.db`, then
  `$HOME/.local/share/metis/metis.db`, then a typed `NoStateDir` error.
- The database file at mode 600 and its directory at mode 700, through the ACL
  rule the previous ticket landed, generalised to take a mode.
- sqlx with runtime queries only: no compile-time macros, no `DATABASE_URL`, no
  committed `.sqlx` offline data, no database server in CI.
- Migrations are committed SQL applied by `sqlx::migrate!` on open, so the
  binary embeds its schema and CI needs no `sqlx-cli` step.
- `sha2` and `getrandom` join the workspace dependencies, and `subtle` provides
  the constant-time hash comparison.
- Token minting and hashing are part of this ticket, so `pairing-and-tokens`
  consumes them instead of adding them.

## Related decisions taken in the same conversation

- ADR 0003 records the state store decision, including the alternatives (a file
  per concern, an embedded key-value store) and the bundled C library cost.
- ADR 0004 records that the daemon does not terminate TLS: a reverse proxy does,
  client trust is deferred past the first efforts, and `tls-and-client-trust` is
  deprecated and out of the graph.
- `pairing-and-tokens` carries the preshared key in the pairing request body,
  keeps token hashes in this store, and the client keeps its token through the
  platform keychain's command line tool when available, else an owner-only file.
