# Testing

> Status: the tooling is in place, but no product behaviour exists yet, so
> there are no meaningful tests. The conventions below are what the bootstrap
> established; revisit them when the first real behaviour lands.

## Framework

- Rust's built-in test harness (`#[test]`), run by `cargo test`.
- The SvelteKit frontend uses `svelte-check` for type and template checking.
  No test runner is configured for it yet.

## Run commands

Run these inside the devenv shell (`devenv shell`):

- `cargo fmt --check` -- formatting.
- `cargo clippy --all-targets -- -D warnings` -- linting; warnings are
  errors.
- `cargo test` -- the full suite across the workspace.

Frontend, run from `apps/ui`:

- `npm run check` -- `svelte-kit sync` plus `svelte-check`.
- `npm run build` -- the static build through `@sveltejs/adapter-static`.

## CI

`.github/workflows/ci.yml` runs the Rust commands above through the devenv
shell on push to `main` and on pull requests. The frontend commands are not
in CI yet; add them when the frontend has behaviour worth gating.

## Mock conventions

- To be decided. Nothing is mocked yet.
