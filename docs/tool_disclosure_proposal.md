# Tool disclosure proposal

## Status and intent

This document proposes the tool disclosure design for Metis Agent. It refines
the compact agent context and tool-permission direction in `docs/rough-plan.md`.
The design separates authorization from the set of tool schemas shown to a
model request, supports a cheap starter selection and a deliberate disclosure
oracle, and remains usable with hosted and small local models.

This proposal is implementation guidance for `metis-agent-core`,
`metis-daemon`, and `metis-plugin-sdk`. The owning effort maps record their
parts of the design.

## Problem

An agent may have permission to call many plugin capabilities, but placing every
tool schema in every model request wastes context and makes selection harder.
For example, a project-management plugin may expose
`{list, view, create, edit, delete}_{project, tag, todo}`, while a request to
create a todo may initially need only `list_todo`, `view_todo`, `create_todo`,
and lookup tools for referenced projects or tags.

Metis needs two paths:

1. A cheap starter path that selects a useful initial toolset before the main
   model call.
2. A deliberate path where the agent can ask a disclosure oracle to search,
   inspect, and choose toolsets or individual tools when the task is more
   complex than the starter selection anticipated.

The selection process must not permanently consume the chat session's model
context. It must not grant permission, hide permission failures, or make
session reconstruction depend on process-local state.

## Goals

- Keep the agent context's system prompt compact and stable.
- Show each model request only the schemas it is likely to need.
- Let the agent deliberately replace or extend its active tool view.
- Support plugin-defined toolsets and selection of individual capabilities.
- Preserve profile permissions as the independent authorization ceiling.
- Keep the design provider-neutral while using native provider tool search
  when it has equivalent semantics.
- Preserve exact reconstruction at every idle boundary.
- Keep workflow Lua steps on exact declared dependencies.

## Non-goals

- Tool disclosure is not a security boundary and does not replace dispatch
  authorization.
- The disclosure oracle does not call business tools or perform external
  effects.
- This design does not give Lua steps dynamic registry access.
- This design does not define plugin installation, trust, or sandboxing.
- This design does not require one ranking algorithm or embedding model.

## Terms

The settled terms are also recorded in `CONTEXT.md`.

- A **toolset** is a named discovery grouping of tool capabilities that are
  useful together for a task or workflow. Membership grants no permission.
- A **tool view** is the exact set of permitted tool schemas visible to one
  model request.
- The **disclosure oracle** is the side-effect-free selection path that searches
  permitted toolsets and tool capabilities for an agent, then changes its tool
  view without adding the selection transcript to model-visible chat history.

## Core rules

### Authorization and disclosure are separate

An agent context's permissions define every tool capability an execution may
call. Disclosure selects a smaller visible subset. For every model request:

```text
visible tools = selected tools intersect agent permissions
```

Tool dispatch checks the agent permissions again. A hidden tool may still be
permitted, and a disclosed tool is not necessarily permitted until the final
check succeeds. Search results never reveal capabilities outside the agent's
permission boundary.

### Tool schemas are structured request data

The harness supplies the active tool view through the provider's structured
tool-definition field. It does not rewrite the original system prompt or insert
JSON schemas as ordinary system prose. Provider adapters may use native
deferred-tool or additional-tool facilities when they preserve the same Metis
semantics.

### Changes happen at idle boundaries

A tool view changes only when no ordinary tool call is running and before the
next model request begins. A disclosure request cannot share a provisional
assistant response with user-visible text or ordinary tool calls. This makes
rollback safe and keeps external effects out of a discarded branch.

### Workflows keep declared dependencies

Regular Lua steps continue to declare exact tool functions. Validation checks
that each function exists and is permitted, and the runtime injects only those
functions. Connectors and fan-in Lua steps receive no tools. Toolsets and the
disclosure oracle apply to model-driven conversations and agent tasks, not to
Lua dependency resolution.

## Tool and toolset metadata

Every tool capability retains its qualified name, description, version, and
request and response schemas. Discovery metadata may add a short use case,
resource, action, tags, and search terms. Descriptions are data for retrieval
and model selection, not instructions that can widen policy.

A plugin may declare reusable toolsets. A toolset has:

- A stable identifier scoped to the plugin or plugin instance.
- A short, discriminating `use_when` description.
- Member capability names.
- Optional related toolsets or individual supporting capabilities.
- Optional tags and search terms.

For example:

```yaml
id: todo.create
use_when: Create a todo, inspect possible duplicates, and resolve its context
tools:
  - todo.list
  - todo.view
  - todo.create
related:
  - project.lookup
  - tag.lookup
```

The daemon validates every member against the registered capability catalog.
It ignores or reports a toolset whose members cannot be resolved according to
the chosen failure policy. A toolset never grants access to a member the agent
context does not permit.

Plugin-provided toolsets are the reusable defaults. Daemon configuration may
define local toolsets or override discovery metadata without changing the
plugin ABI. The exact backward-compatible serialized representation remains an
implementation decision shared by the daemon and plugin SDK efforts.

## Active tool view

The active tool view contains:

- Pinned intrinsic tools needed for control, including the capability request
  mechanism when the oracle is enabled.
- Tools selected by the starter path.
- Tools or toolsets selected by the disclosure oracle.
- A catalog revision and stable schema identities needed for reconstruction.

Selection supports two explicit modes:

- `replace`: replace the non-pinned active selection.
- `add`: add selected tools to the current view.

Pinned intrinsic tools survive either mode. The daemon applies an active-schema
budget and deterministic ordering. A later implementation may add eviction, but
it must be explicit and reconstructable rather than silently dropping schemas.

## Starter selection

Before the first main model request, and optionally after a clear topic change,
the daemon selects a useful starting point from the permitted catalog. It may
use structured resource and action metadata, lexical matching, embeddings, or a
small selector model. The mechanism is outside model-visible chat history.

Starter selection prefers toolsets because they encode relationships that raw
similarity cannot infer. It may add individual supporting tools. Low confidence
falls back to pinned tools rather than exposing the complete catalog.

Starter selection is an optimization. Failure to retrieve the best toolset does
not change permissions and can be corrected by the disclosure oracle.

## Disclosure oracle

### Request

The main agent receives a small intrinsic control capability equivalent to:

```yaml
request_capabilities:
  query: string
  mode: replace | add
```

Calling it is control flow rather than an ordinary business-tool call. The
assistant response containing the call remains provisional and is not committed
to the chat session.

### Ephemeral selection branch

The harness starts a side branch from the last committed session state. The
branch may use only read-only catalog operations such as:

- Search toolsets.
- Inspect a toolset's members, use case, and approximate schema cost.
- Search individual tools.
- Submit one structured selection decision.

Search may combine lexical ranking, structured metadata, and embeddings. The
oracle may use the main model or a consumer-selected smaller model. It receives
only capabilities already filtered through the agent's permissions.

Ranked results initially contain compact summaries rather than full schemas.
The oracle can inspect promising results and select several toolsets, several
individual tools, or both, subject to the configured schema budget.

### Commit and resume

When selection succeeds:

1. The provisional assistant response and oracle transcript are excluded from
   model-visible chat history.
2. The harness records a tool-view-change persistence event.
3. The active tool view changes at the idle boundary.
4. The harness repeats the original model request from the same committed
   conversation state with the new structured tool definitions.

The model therefore continues from the point before it requested disclosure,
but it sees the selected tools. The original system prompt is unchanged.

The oracle transcript may be retained in a separate operational trace for
diagnostics. It is not part of the reconstructed model conversation.

### Safety and loop limits

- The oracle branch cannot call ordinary plugin tools.
- A disclosure request must be the only action in its provisional response.
- Each model step has a bounded number of disclosure attempts.
- Invalid selections, unavailable members, and budget overflow are typed
  failures and leave the previous view intact unless an explicit fallback is
  configured.
- Repeated equivalent requests may reuse a cached selection if the permission
  and catalog revisions still match.

## Persistence and reconstruction

The harness persists the result of disclosure, not the model-visible oracle
transcript. A persistence event contains at least:

- The disclosure source, such as starter, oracle, or explicit consumer policy.
- The `replace` or `add` mode.
- Selected toolset and individual capability identifiers.
- The resulting ordered tool view.
- The permission and catalog revisions.
- Stable schema versions or digests.

Reload must reproduce the same active tool view. If a required plugin or schema
revision is unavailable, loading fails loudly or enters an explicitly designed
compatibility path. It does not silently bind the old identifier to a changed
schema.

The store acknowledges the tool-view-change event under the same idle-boundary
rule as other session state. An idle session is not safe to discard before the
event is saved.

## Provider adaptation and caching

The Metis semantics are provider-neutral:

- A provider with native deferred tool search may use it when the adapter can
  enforce the permission filter, active-view semantics, persistence identity,
  and replacement behavior.
- A provider without native support receives the resolved structured tool list
  on each model request.
- A native provider's search records may exist in provider-specific transport
  history, but they do not become canonical Metis chat messages.

The stable agent prompt and committed chat history remain an unchanged prefix.
New tool schemas are appended or supplied through the provider's structured
tool channel where possible. Tool ordering is deterministic. Provider cache
behavior is an optimization and never changes reconstructed session semantics.

## Ownership

### `metis-agent-core`

- Defines the provider-neutral tool view and disclosure request and decision
  types.
- Owns provisional responses, the ephemeral branch, commit and rollback, idle
  boundaries, attempt limits, lifecycle events, and exact reconstruction.
- Accepts consumer-provided catalog, starter-selection, oracle-model, and
  persistence interfaces without depending on plugins or daemon configuration.
- Keeps workflow Lua tool injection on exact declared dependencies.

### `metis-daemon`

- Builds the catalog from registered plugin capabilities and configured
  toolsets.
- Filters the catalog through agent permissions before any search.
- Implements starter retrieval, the concrete oracle, ranking indexes, schema
  budgets, and profile disclosure policy.
- Adapts native provider tool search to the harness contract where possible.
- Persists disclosure events through its `SessionStore` implementation and
  diagnoses missing catalog revisions during reload.

### `metis-plugin-sdk`

- Gives Rust authors ergonomic declarations for discovery metadata and
  reusable toolsets.
- Validates that toolset members name declared capabilities.
- Generates the agreed bundle or manifest representation and documents the
  equivalent hand-authored representation for non-Rust plugins.
- Does not implement selection, ranking, permissions, or the oracle.

## Open implementation decisions

- Exact Rust traits and event types.
- The first serialized toolset representation and its compatibility rules.
- Whether local toolset overlays replace or extend plugin declarations.
- Default starter-selection confidence, schema budget, and oracle-attempt
  limits.
- The ranking and reranking algorithms and how their indexes are refreshed.
- How long an active tool view remains sticky across topic changes.
- The provider-specific equivalence tests required before native tool search
  can replace the generic oracle path.
- Whether surfaces expose the active tool view and operational oracle trace.

## Required tests

- A starter selection exposes only permitted members of the selected toolset.
- An oracle can combine toolsets and individual tools in `replace` and `add`
  modes.
- The provisional disclosure response and oracle transcript do not enter the
  model-visible chat history.
- Ordinary tool calls and user-visible text cannot share a disclosure request.
- Dispatch rejects a capability removed from the permission boundary after
  selection.
- Save, drop, reload, and continue reproduce the exact ordered tool view.
- A missing or changed schema revision fails according to the declared policy.
- Workflow Lua steps still receive only their exact declared permitted tools.
- Native and generic provider adapters produce equivalent disclosure behavior.
- Attempt and schema budgets prevent disclosure loops and unbounded context
  growth.

