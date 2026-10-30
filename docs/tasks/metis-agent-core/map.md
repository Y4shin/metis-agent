---
type: map
title: Metis agent core
status: stable
blocked_by: []
---

# Metis agent core

## Destination

The reusable agent runtime exists as two independent crates. `agent-harness`
runs the agent loop with provider and tool contracts supplied by its consumer,
constructs and transforms context, and exposes both ongoing conversations and
bounded agent tasks through the same loop, with lifecycle events,
cancellation, execution limits, and extension hooks. Session state satisfies
the exact-reconstruction contract at every idle boundary through the
`SessionStore` trait. The `workflow` crate defines and executes directed
acyclic workflows with input and output schemas, named steps, and Lua
connector routing, covering exactly three step types: agentic task, Lua step,
and fan-in Lua step.

## Constraints

- `agent-harness` depends on no Metis network contract, plugin loader, daemon
  configuration, or UI. Tools and hooks arrive through Rust interfaces.
- Concrete provider implementations live with the consumer, not in the
  harness.
- Context selected for one model request is derived from session state;
  pruning or transforming model input never discards session history.
- Persisting state is the consumer's job; the harness owns session types,
  persistence-event semantics, and reconstruction. Persistence failures are
  surfaced, never reported as saved.
- The `workflow` crate keeps storage backends, scheduling, network transports,
  and concrete integrations outside itself. Consumers supply them.
- A task or step may narrow the enclosing agent context's tools but never
  widen them.
- Lua steps get only their declared, permitted tool functions. No full tool
  registry, no unrestricted dynamic lookup, no filesystem, network, process,
  or native access through Lua globals or imports.
- Connectors and fan-in Lua steps receive no tool functions. Routing functions
  transform and route, they do not call tools.

## Decisions so far

- From `docs/rough-plan.md`, "agent-harness": model and tool contracts only,
  the loop, context transformation, lifecycle events, cancellation, limits,
  hooks.
- From "Conversations and agent tasks": one loop serves both; tasks support
  declared output schemas and validation; a task is bounded but may span many
  model turns.
- From "Session state and storage contract": `SessionStore` trait, streamed
  ordered persistence events, exact reconstruction at every idle boundary.
- From "workflow", "Definition and routing", "Join readiness and unreachable
  branches", and "Declared tool dependencies": DAGs, three step types,
  connector Lua routing with conditional fan-out, joins that wait for value
  or confirmed unreachability, declared tool dependencies.
- From "Mandatory agent context": every conversation, task, and workflow run
  belongs to an agent context; workflows inherit permissions but not the
  system prompt.

## Tool disclosure responsibilities

`docs/tool_disclosure_proposal.md` defines progressive tool disclosure for
model-driven conversations and agent tasks. This effort owns the reusable,
provider-neutral mechanism:

- Define the active tool view and the disclosure request, decision, and
  persistence-event types.
- Support starter-selected tools plus an agent-requested disclosure oracle
  that may replace or add to the non-pinned view.
- Keep the assistant response that requests disclosure provisional, run the
  side-effect-free oracle branch from the last committed state, exclude that
  branch from model-visible chat history, persist the resulting view change,
  and repeat the original model request with the selected structured schemas.
- Change a tool view only at an idle boundary. A disclosure request cannot
  share a provisional response with ordinary tool calls or user-visible text.
- Preserve exact reconstruction of the ordered tool view through stable
  schema identities and catalog and permission revisions.
- Accept consumer-provided catalog, starter-selection, oracle-model, and
  persistence interfaces. The harness does not depend on plugin metadata,
  embeddings, daemon configuration, or one provider's native tool search.
- Keep Lua workflow steps on exact declared dependencies. They do not use the
  disclosure oracle or unrestricted dynamic lookup.

Tool visibility never grants permission. The consumer supplies an
authorization-filtered catalog, and dispatch still enforces the enclosing
agent context's permissions.

## Fog

- The exact Rust interfaces of the loop, provider contract, tool contract,
  and hook points.
- Agent-task output contract representation and validation.
- Workflow definition format: where it is written, and how step and port keys
  are represented.
- Mandatory-port connectivity rules, dead-path detection, and how failed or
  cancelled branches affect join readiness, as distinct from deliberate
  abandonment.
- Execution concurrency in the workflow runtime.
- Durable checkpoint and resume semantics, and the failure and retry policy,
  including idempotency or reconciliation for external writes.
- The Lua runtime and sandbox, and the exact tool-injection API.
- Scheduling, model-provider selection, and storage details. This effort owns
  the interfaces; the implementations belong to `metis-daemon`.

## Out of scope

- The network contract, generated types, client, and the realtime design are
  owned by `metis-foundations`. It proves the transports end to end with one
  dummy request and one dummy event. The real endpoints are implemented in
  `metis-daemon`, not here.
- The plugin contract and runtime, including plugin tool adapters.
- The daemon composition layer: IPC, configuration, and policy.
- Network contracts, generated types, and the client crate.
- Any UI or terminal application.
