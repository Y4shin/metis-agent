---
type: arch spec
title: arch-spec, metis-bootstrap
status: stable
---

# arch-spec, metis-bootstrap

> This is a scaffolding task, not TDD-able application code. The "seams" are
> workspace-level build, format, lint, and test commands. There is no code
> interface contract between the tickets, and no public Rust API surface; the
> "interface contract" is the on-disk layout each ticket leaves behind for the
> five followup efforts.

## Source of truth for the target state

- Architecture: `docs/rough-plan.md`
- Effort decisions: `docs/tasks/metis-bootstrap/map.md` (`D1` through `D9`)
- Spec: `docs/tasks/metis-bootstrap/spec.md`
- Tickets: `docs/tasks/metis-bootstrap/tickets/`

## Exports

This effort exports no Rust API. What it exports is the workspace shape:

- `crates/<name>` directories registered as workspace members, with path
  dependencies encoding the dependency direction from `docs/rough-plan.md`.
  Package names match directory names; libraries expose an empty `lib.rs` and
  binaries expose only `fn main() {}`.
- `contracts/openapi.yaml`, `contracts/asyncapi.yaml`, and
  `contracts/schemas/`.
- `apps/ui`, a SvelteKit project with `@sveltejs/adapter-static`.
- Tooling: `rustfmt.toml`, `.editorconfig`, `.github/workflows/ci.yml`, and the
  AGPL-3.0 `LICENSE`.
- Five followup map skeletons under `docs/tasks/`, registered in
  `docs/tasks/index.md`.

## Existing abstractions to use

- devenv (`devenv.nix`) is the toolchain source: Rust nightly through
  rust-overlay, plus Node.js and npm. Nothing else pins the toolchain.
- The root `Cargo.toml` workspace uses `resolver = "3"` and edition 2024.
- The SvelteKit creator (`sv create`) and adapter-static own the frontend
  scaffold; do not replace generated files by hand.

## Do NOT reimplement

- Do not hand-write the SvelteKit project when the creator covers it.
- Do not add a `rust-toolchain.toml`. devenv pins the toolchain, and the file
  waits until CI builds release binaries.
- Do not create the optional `config` or `plugin-sdk` crates.
- Do not add behaviour, module structure, types, or API surface to any crate.
- Do not restate `docs/rough-plan.md` inside the followup maps; reference it.

## Seams (verification contract)

Each seam is a command run inside the devenv shell unless noted:

1. `cargo check --workspace` green.
2. `cargo fmt --check` green.
3. `cargo clippy --all-targets -- -D warnings` green.
4. `cargo test` green.
5. `apps/ui`: `npm run check` and `npm run build` green (npm, outside the
   shell is fine).
6. `tw_frontier metis-bootstrap` and `tw_map_finalizable metis-bootstrap`
   resolve with no anomalies.

## Ticket interface contracts

- `scaffold-repo-skeleton` leaves the workspace, crates, contracts, `apps/ui`,
  and tooling. Contract for later efforts: crate directories, package names,
  and path dependencies are fixed, and followup efforts add code inside them
  rather than reshaping the workspace.
- `create-followup-efforts` leaves the five `docs/tasks/<effort>/map.md`
  skeletons and their index entries. Contract for later efforts: each map
  stays low-resolution and owns its own frontier in a later Wayfinder session.
- The two tickets carry no ordering edge; either can start first.

## Architecture notes

- Execution is parent-session, with no subagent chains. The tickets carry
  `mode: human` so the router never dispatches a chain, and their bodies record
  that the parent agent executes them itself.
- The scaffold was implemented and verified in the parent session. `cargo
  check`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test`, and `apps/ui` `npm run check` and `npm run build` all pass.
- The workspace lint block allows `cargo::unused_dependencies` because the path
  dependencies are wired before any crate has code that consumes them.
