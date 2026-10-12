---
type: ticket
subtype: feature
title: Create the five followup effort maps
status: deprecated
workflow_state: done
blocked_by: []
size: s
mode: human
---

## What to build

Five followup efforts exist under `docs/tasks/` as Wayfinder map skeletons,
each registered in `docs/tasks/index.md`. Each map carries the four required
sections and frontmatter, and each is low-resolution by design: destination,
constraints, decisions inherited from `docs/rough-plan.md`, fog, and
out-of-scope. No decision tasks are written into them; each followup's own
Wayfinder session charts its frontier.

The five efforts:

- `metis-foundations`: the network and plugin contracts (OpenAPI, AsyncAPI,
  plugin C ABI), the generated Rust types, and a thin end-to-end
  daemon/client/terminal skeleton that proves HTTP and WebSocket over the
  local IPC transports.
- `metis-agent-core`: the `agent-harness` crate (agent loop, provider and tool
  contracts, session state and the exact-reconstruction storage contract,
  conversations and bounded tasks) and the `workflow` crate (DAG definition,
  routing, joins, restricted Lua steps).
- `metis-plugins`: the host side of the plugin system: the versioned C ABI and
  manifest contract, bundle format, loading, compatibility checks, registry,
  capability discovery, dispatch, callbacks, cancellation, and lifetime.
- `metis-surfaces`: the Tauri 2 desktop shell, the shared SvelteKit frontend,
  and the Android shell and assistant integration, all through the Rust
  client.
- `metis-plugin-sdk`: the plugin authoring SDK (Rust trait and export
  adapters), a set of core plugins, and a plugin CLI.

## Acceptance criteria

- [ ] Each of the five map files exists at `docs/tasks/<effort>/map.md` with
      conformant frontmatter (`type: map`, a title, and a status).
- [ ] Each map body carries Destination, Constraints, Decisions so far, Fog,
      and Out of scope.
- [ ] Each map's decisions reference the relevant sections of
      `docs/rough-plan.md` rather than restating them wholesale.
- [ ] `metis-plugin-sdk` is scoped to the SDK, the core plugin set, and the
      CLI, and `metis-plugins` is scoped to the host side, with the boundary
      stated on both maps.
- [ ] All five slugs appear in the `## Live` list of `docs/tasks/index.md`,
      sorted.
- [ ] `tw_frontier` and `tw_dependency_levels` resolve for each new effort.
- [ ] No decision tasks are written into any of the five maps.

## Execution note

This ticket is `mode: human` so that no subagent chain is dispatched. The
parent agent executes it directly in-session; the human has authorised that.
Do not delegate this ticket to subagents.

## Blocked by

- None (can start immediately).

## Implementation notes

Delivered by the parent session with no subagent delegation, on `main`.

- Five map skeletons exist and are registered in `docs/tasks/index.md`: 
  `metis-foundations`, `metis-agent-core`, `metis-plugins`, `metis-surfaces`,
  `metis-plugin-sdk`.
- Effort-to-effort edges were added beyond the minimum: `metis-plugins` and
  `metis-surfaces` are blocked by `metis-foundations`, and
  `metis-plugin-sdk` is blocked by `metis-plugins`. `tw_dependency_levels`
  resolves them.
- The fifth effort is the plugin SDK deliverable itself (SDK, core plugins,
  CLI), not a separate planner map.
- No decision tasks were written into any of the five maps; each followup
  charts its own frontier in a later Wayfinder session.
