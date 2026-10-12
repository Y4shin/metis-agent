---
type: map
title: Metis bootstrap
status: stable
blocked_by: []
---

# Metis bootstrap

## Destination

The repository carries the complete project skeleton from `docs/rough-plan.md`
and nothing more: a Cargo workspace with all nine agreed crates, a
`contracts/` tree, an `apps/ui` SvelteKit project, and the agreed tooling. No
product code exists yet. Five followup efforts exist as Wayfinder maps under
`docs/tasks/`, each a low-resolution skeleton for its own later Wayfinder
session.

## Constraints

- Scaffolds only. An empty `src/lib.rs`, or a bare `fn main() {}` in a binary,
  is allowed; any behavioural stub, provider implementation, or hand-written
  API surface is not.
- The `config` and `plugin-sdk` crates are not created here. `plugin-sdk` is
  its own followup effort.
- The Rust toolchain comes from devenv (nightly via `rust-overlay`). No
  `rust-toolchain.toml` is added yet; it arrives when CI starts building
  release binaries.
- Edition 2024, workspace resolver 3. Crate package names match their
  directory names, with no `metis-` prefix.
- `apps/ui` uses npm and `@sveltejs/adapter-static`, and contains no daemon
  or backend logic. All backend behaviour stays in Rust.
- The five followup maps are skeletons: destination, constraints, decisions
  already inherited from `docs/rough-plan.md`, and fog. Each gets its own
  Wayfinder session later.
- Execution is HITL. The bootstrap tickets carry `mode: human` so no subagent
  chain is dispatched; the parent agent executes them directly in-session.
- No em-dashes in any document this effort produces.

## Decisions so far

- **D1, decomposition.** The implementation of `docs/rough-plan.md` is sliced
  walking-skeleton first, not crate by crate. Five followup efforts:
  `metis-foundations`, `metis-agent-core`, `metis-plugins`, `metis-surfaces`,
  and `metis-plugin-sdk`.
- **D2, scaffold scope.** Create all nine agreed crates (`agent-harness`,
  `workflow`, `plugin-contract`, `plugin-runtime`, `api-contracts`, `client`,
  `daemon`, `terminal`, `app`), the `contracts/` tree with placeholder
  `openapi.yaml`, `asyncapi.yaml`, and `schemas/`, and the `apps/ui`
  SvelteKit project. Leave the optional `config` crate out. Leave
  `crates/app`'s source empty rather than stubbing a Tauri entry point, so
  Tauri's dependency weight does not land in the bootstrap.
- **D3, tooling.** Add `rustfmt.toml`, a clippy lint block in the workspace
  `Cargo.toml`, `.editorconfig`, a GitHub Actions CI workflow running `fmt`,
  `clippy`, and `test`, the AGPL-3.0 `LICENSE` text (the README already
  declares that licence but the file does not exist), and the
  ESLint + Prettier + `svelte-check` baseline for `apps/ui`. Defer
  `rust-toolchain.toml` until CI builds release binaries.
- **D4, execution path.** The bootstrap is its own `to-spec` then
  `to-tickets` effort, executed by `/skill:implement-task`. It is not carried
  in the map itself.
- **D5, ticket mode.** Both bootstrap tickets are `mode: human`. The parent
  agent executes them itself without delegating to subagents.
- **D6, fifth followup.** The fifth followup effort is `metis-plugin-sdk`,
  whose destination is to deliver a plugin authoring SDK, a set of core
  plugins, and a plugin CLI. There is no separate planner map.
- **D7, UI scaffold.** `apps/ui` is created with `npx sv create` using the
  minimal template, TypeScript, ESLint + Prettier + `svelte-check`, npm, and
  `@sveltejs/adapter-static`.
- **D8, crate scaffold.** Explicit `members` list in the root `Cargo.toml`
  (no `crates/*` glob), bare package names, edition 2024, path dependencies
  wired to match the dependency diagram in `docs/rough-plan.md`.
- **D9, followup maps.** The five followup maps are written by a manual
  ticket in this effort, not charted directly by this Wayfinder run.

## Fog

- Whether `contracts/openapi.yaml` and `contracts/asyncapi.yaml` are valid
  minimal documents or empty placeholders. Resolve in `metis-foundations`.
- Exact generator tooling for the contracts, generated-code placement, and
  regeneration checks. Resolve in `metis-foundations`.
- CI specifics beyond `fmt`, `clippy`, `test`: caching, matrix, devenv use in
  Actions, and when the release-binary job lands. Some of this depends on
  `rust-toolchain.toml`, deferred by D3.
- Which of the rough plan's "Open decisions" belong to which followup effort.
  Each followup's own Wayfinder session decides its frontier.

## Out of scope

- Any implementation code for any crate or app.
- The plugin authoring SDK, the core plugin set, and the plugin CLI, which
  belong to `metis-plugin-sdk`.
- The optional `config` crate and the optional `plugin-sdk` crate, both
  deferred by `docs/rough-plan.md` until their boundaries prove useful.
- Android scaffolding and Tauri wiring, which belong to `metis-surfaces`.
- Provider implementations, storage backends, and UI screens.
