---
type: ticket
subtype: feature
title: Scaffold the Metis repository skeleton
status: deprecated
workflow_state: done
blocked_by: []
size: m
mode: human
---

## What to build

The repository gains the complete project skeleton described in
`docs/tasks/metis-bootstrap/spec.md` and `docs/rough-plan.md`, with no product
code. When this ticket is done, a person can enter the devenv shell, run the
workspace checks, build the empty crates, and open the SvelteKit project,
without any crate containing behaviour.

The work covers:

- the root Cargo workspace with the nine agreed crates under `crates/`, wired
  to the agreed dependency direction;
- the `contracts/` tree with placeholder OpenAPI and AsyncAPI documents and an
  empty `schemas/` directory;
- the `apps/ui` SvelteKit project with TypeScript, ESLint, Prettier,
  `svelte-check`, npm, and `@sveltejs/adapter-static`;
- the shared tooling: `rustfmt.toml`, a workspace clippy lint block,
  `.editorconfig`, a GitHub Actions workflow for format, lint, and test, and
  the AGPL-3.0 `LICENSE` text;
- Node.js and npm added to the devenv shell.

## Acceptance criteria

- [ ] The nine crates exist as workspace members: `agent-harness`, `workflow`,
      `plugin-contract`, `plugin-runtime`, `api-contracts`, `client`,
      `daemon`, `terminal`, `app`.
- [ ] Library crates have an empty `src/lib.rs`; `daemon`, `terminal`, and
      `app` have a `src/main.rs` holding only a bare `fn main() {}`. No module
      structure, types, or behaviour exist.
- [ ] Inter-crate path dependencies match the dependency direction in
      `docs/rough-plan.md`.
- [ ] `contracts/openapi.yaml` and `contracts/asyncapi.yaml` are minimal valid
      placeholder documents, and `contracts/schemas/` exists.
- [ ] `apps/ui` is a SvelteKit project with the agreed tooling and adapter,
      and contains no backend logic.
- [ ] `rustfmt.toml`, the clippy lint block, `.editorconfig`, the CI workflow,
      and the AGPL-3.0 `LICENSE` text are present.
- [ ] `devenv.nix` provides Node.js and npm alongside Rust.
- [ ] Inside the devenv shell, `cargo check --workspace`, `cargo fmt --check`,
      `cargo clippy --all-targets -- -D warnings`, and `cargo test` all pass.
- [ ] Inside `apps/ui`, `npm run check` and `npm run build` pass.
- [ ] No `rust-toolchain.toml` is added.
- [ ] The optional `config` and `plugin-sdk` crates are not created.

## Execution note

This ticket is `mode: human` so that no subagent chain is dispatched. The
parent agent executes it directly in-session; the human has authorised that.
Do not delegate this ticket to subagents.

## Blocked by

- None (can start immediately).

## Implementation notes

Delivered by the parent session with no subagent delegation, on `main`.

- All nine crates, `contracts/`, `apps/ui`, and the tooling are in place, and
  every acceptance check was run and passed: `cargo check --workspace`,
  `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test`, plus `npm run check` and `npm run build` in `apps/ui`.
- Binaries carry a bare `fn main() {}` rather than a truly empty `src/main.rs`,
  because a Rust binary needs an entry point to compile. The acceptance
  criterion was corrected to match.
- `apps/ui` uses `@sveltejs/adapter-static` with `fallback: 'index.html'`.
  Strict prerendering rejects the generated dynamic root route, and the shell
  is a single-page app that reaches the daemon through the Rust client.
- The workspace lint block allows `cargo::unused_dependencies` because the
  inter-crate path dependencies are wired before any crate has code that
  consumes them. Flip it back on once real code lands.
- `LICENSE` was added: `README.md` already declared AGPL-3.0, but the file did
  not exist.
- CI runs the Rust checks through the devenv shell. The frontend checks are
  not in CI yet.
- No `rust-toolchain.toml`; devenv remains the single toolchain source until CI
  builds release binaries.
