---
type: spec
title: Metis bootstrap
status: stable
---

# Metis bootstrap

## Problem Statement

`docs/rough-plan.md` records the agreed architecture for Metis Agent, but the
repository holds only a devenv shell, an empty Cargo workspace, and no effort
graph. There is nowhere for implementation work to land: no crates, no
contracts tree, no frontend project, and no tooling to keep the workspace
honest. Planning the implementation cannot start until the shape of the
repository exists and the followup efforts are named.

## Solution

Create the full project skeleton described in `docs/rough-plan.md` without
writing any product code, and register five followup efforts as Wayfinder map
skeletons so each can be planned in its own session. The skeleton is the
workspace, the crate layout, the contracts tree, the SvelteKit frontend, the
shared tooling, and the licence. The followups decompose the architecture:
foundations and contracts, the agent core, the plugin host, the application
surfaces, and the plugin authoring SDK with its core plugins and CLI.

## User Stories

1. As the maintainer, I want every agreed crate to exist as an empty
   workspace member, so that each followup effort has a stable home.
2. As the maintainer, I want the inter-crate path dependencies wired to match
   the agreed dependency direction, so that a layering mistake is visible at
   the workspace level.
3. As the maintainer, I want a `contracts/` tree with placeholder OpenAPI and
   AsyncAPI documents, so that the network contract work has an authoritative
   location from day one.
4. As the maintainer, I want an `apps/ui` SvelteKit project, so that the
   frontend followup starts from a real project instead of a blank directory.
5. As the maintainer, I want a reproducible development shell, so that Rust,
   Node, and the frontend tooling come from one pinned environment.
6. As the maintainer, I want formatting, lint, and CI configuration in place
   before code lands, so that quality checks are never a later retrofit.
7. As the maintainer, I want the AGPL-3.0 licence text committed, so that the
   repository matches the licence its README declares.
8. As the maintainer, I want a workspace-wide build to succeed on an empty
   skeleton, so that the bootstrap itself is verifiable.
9. As the maintainer, I want five followup efforts registered as maps, so that
   planning can proceed effort by effort instead of as one foggy blob.
10. As an implementing agent, I want each followup map to carry a destination,
    constraints, and inherited decisions, so that its Wayfinder session starts
    from the repository's shared understanding.
11. As an implementing agent, I want the bootstrap tickets marked human-owned,
    so that no subagent chain is dispatched for mechanical file creation.
12. As the maintainer, I want the frontend project to contain no backend
    logic, so that the boundary between SvelteKit and the Rust daemon stays
    intact.
13. As the maintainer, I want the optional crates from the rough plan to stay
    uncreated, so that the skeleton does not encode decisions the architecture
    has not made.
14. As the maintainer, I want the toolchain pinned only where it is needed, so
    that devenv and a future CI binary job do not fight over the version.

## Implementation Decisions

- **Workspace layout.** One root `Cargo.toml` with an explicit `members` list.
  Nine members under `crates/`: `agent-harness`, `workflow`,
  `plugin-contract`, `plugin-runtime`, `api-contracts`, `client`, `daemon`,
  `terminal`, `app`. Package names match directory names, edition 2024,
  resolver 3.
- **Crate targets.** `agent-harness`, `workflow`, `plugin-contract`,
  `plugin-runtime`, `api-contracts`, and `client` are libraries with an empty
  `src/lib.rs`. `daemon`, `terminal`, and `app` are binaries whose `src/main.rs`
  holds only a bare `fn main() {}`, the minimum needed for the workspace to
  build. No module structure, no types, no behaviour.
- **Dependency edges.** Path dependencies are wired so the workspace encodes
  the rough plan's dependency direction: `daemon` depends on `agent-harness`,
  `workflow`, `plugin-runtime`, and `api-contracts`; `workflow` depends on
  `agent-harness`; `plugin-runtime` depends on `plugin-contract`; `terminal`
  and `app` depend on `client`; `client` depends on `api-contracts`. The
  `api-contracts` crate does not yet depend on generated code.
- **Contracts tree.** `contracts/openapi.yaml` and `contracts/asyncapi.yaml`
  are minimal valid placeholder documents with version and info blocks and no
  paths or channels. `contracts/schemas/` is created empty. Whether the
  placeholders become minimal valid specs is a decision deferred to
  `metis-foundations`; the bootstrap only guarantees the tree exists.
- **Frontend.** `apps/ui` is generated with the standard SvelteKit creator
  using the minimal template, TypeScript, ESLint, Prettier, `svelte-check`,
  npm, and `@sveltejs/adapter-static`. No application code, no routes beyond
  the template, no Tauri wiring.
- **Tooling.** `rustfmt.toml`, a workspace clippy lint block, `.editorconfig`,
  and a GitHub Actions workflow running `cargo fmt --check`,
  `cargo clippy --all-targets -- -D warnings`, and `cargo test`. The
  AGPL-3.0 `LICENSE` text is added because the README declares that licence.
  No `rust-toolchain.toml`; devenv remains the toolchain source.
- **Development shell.** `devenv.nix` gains Node.js and npm so the frontend
  has a pinned toolchain alongside Rust, and the frontend project runs inside
  the same shell as the crates.
- **Followup maps.** Five maps are created under `docs/tasks/`:
  `metis-foundations`, `metis-agent-core`, `metis-plugins`, `metis-surfaces`,
  `metis-plugin-sdk`. Each carries destination, constraints, decisions
  inherited from the rough plan, fog, and out-of-scope, and each is registered
  in `docs/tasks/index.md`. No decision tasks are written into them; each
  followup's own Wayfinder session charts its frontier.
- **Boundaries.** `metis-plugin-sdk` owns the authoring SDK, the core plugin
  set, and the plugin CLI. `metis-plugins` owns the host side: contract ABI,
  bundle format, loading, dispatch, and lifecycle.
- **Execution.** The bootstrap is not carried in the map. It proceeds through
  a spec and tickets, and the tickets are `mode: human` so that the parent
  agent executes them without subagent delegation.

## Testing Decisions

- The bootstrap's external behaviour is "an empty workspace builds and the
  checks pass". That is tested by running, inside the devenv shell:
  `cargo check --workspace`, `cargo fmt --check`, `cargo clippy --all-targets
  -- -D warnings`, and `cargo test`. A bootstrap is done when those are green.
- The frontend's external behaviour is "the scaffold builds". That is tested
  by running the generated project's own `npm run check` and `npm run build`
  inside `apps/ui`.
- No unit tests are written, because no behaviour exists. Tests arrive with
  the first real behaviour in `metis-foundations`.
- The crate graph itself is verified structurally: every declared member
  exists, every path dependency resolves, and `cargo metadata` succeeds.
- Acceptance is manual for the effort graph: the five followup maps exist,
  parse, are registered in the index, and each carries the required sections.

## Out of Scope

- Any product code, module structure, trait, type, or API surface.
- Provider implementations, storage backends, scheduling, authentication, and
  configuration resolution.
- The plugin authoring SDK, the core plugin set, and the plugin CLI.
- Tauri wiring, Android scaffolding, and any UI screen beyond the template.
- The optional `config` and `plugin-sdk` crates.
- CI beyond formatting, linting, and tests; release-binary jobs wait for
  `rust-toolchain.toml`.

## Further Notes

- The five followup efforts are planned walking-skeleton first. The intention
  is that `metis-foundations` proves the riskiest interface assumptions, the
  IPC transports and the contract tooling, before the remaining efforts build
  on top.
- `docs/rough-plan.md` remains the architectural source of record for now. Its
  "Open decisions and early validation" list is the raw material each
  followup's Wayfinder session draws its frontier from.
- Vocabulary stays aligned with `CONTEXT.md`: use Daemon, Plugin, Chat
  session, and Small local model as defined there when the followups are
  written.
