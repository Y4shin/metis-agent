# Metis Agent architecture

This document records the architecture agreed during initial project planning. Use it as implementation guidance. Decisions explicitly marked open require further design; examples are not finalized APIs.

## Product and priorities

Metis Agent is a minimal, extensible personal AI assistant built around a persistent daemon. The daemon runs locally on a personal workstation or on a server for access from multiple devices. It uses tools to carry out tasks across users' workflows, including work that continues beyond an individual chat session.

The priorities are simplicity, extensibility, customization, and efficient prompts and context usage, particularly for smaller local models. Metis is a general personal assistant rather than exclusively a coding agent. Capabilities are shaped through plugins.

## Agreed technology choices

- Rust for the daemon, agent harness, plugin host, network client, terminal application, and native application logic.
- A self-written agent harness. Pi's core runtime is an architectural inspiration, not a runtime dependency or a requirement to port its code.
- Support both ongoing conversations and bounded agent tasks. A separate reusable `workflow` crate composes agent tasks and runtime-provided Lua steps into explicit DAGs, using models selectively where they are needed.
- A self-written plugin system using compressed bundles containing a manifest, assets as needed, and exactly one native shared library (DLL) per plugin exposing a versioned C ABI, with Rust abstractions over that ABI.
- OpenAPI for ordinary HTTP requests and AsyncAPI for WebSocket realtime communication.
- HTTP and WebSockets over local IPC on all three desktop platforms: Unix domain sockets on Linux and macOS/Darwin, and duplex byte-mode named pipes on Windows. Remote access uses an explicitly enabled TCP/TLS listener.
- Tauri 2 for both desktop and Android application shells.
- SvelteKit for the shared application frontend, statically built for Tauri. Backend functionality belongs in Rust, not SvelteKit server routes.
- Custom Kotlin/Java Android integration where needed, including system assistant integration.
- One Rust crate containing both CLI commands and an interactive TUI.
- No iOS implementation is currently planned.

## Mandatory agent context

The daemon declares multiple agents (working term; final naming remains open). Each definition consists of a compact system prompt and permissions, initially including the tools it may call. Keep the prompt low-level and concise to minimize system-prompt token overhead. Detailed declaration formats and additional permission categories remain open.

Every conversation, standalone agent task, and workflow run must belong to an agent context. Conversations use its system prompt and tool permissions. Workflows use its permissions to determine available tools; they do not automatically inherit its system prompt. Agent-task steps use their own task instructions within that permission boundary. A task or step may narrow available tools but must not widen the enclosing agent's permissions.

The daemon owns these definitions and resolves them into prompt/tool policy supplied to the reusable harness and workflow runtime. Multiple configured agents do not imply autonomous delegation or inter-agent messaging; those behaviors are not specified here.

## Repository layout

The repository root is named `metis-agent`. Proposed directory layout:

```text
metis-agent/
├── Cargo.toml
├── contracts/
│   ├── openapi.yaml
│   ├── asyncapi.yaml
│   └── schemas/
├── crates/
│   ├── agent-harness/
│   ├── workflow/
│   ├── plugin-contract/
│   ├── plugin-runtime/
│   ├── api-contracts/
│   ├── client/
│   ├── daemon/
│   ├── terminal/
│   └── app/
└── apps/
    └── ui/                 # SvelteKit frontend
```

Directory and Cargo package names can differ. Published package naming has not been finalized. Optional crates described below should only be created when their boundaries are useful.

## Rust crate responsibilities

### `agent-harness`

A reusable harness library containing both execution and its extension interfaces:

- Model/provider and tool contracts only; concrete provider implementations are the responsibility of the importer/consumer of the `agent-harness` crate.
- The agent loop: prepare a request, call a model, execute requested tools, record results, and continue or finish.
- Context construction and transformation, with explicit control over instructions, messages, tool schemas, and tool results sent to the model.
- Lifecycle events, cancellation, execution limits, and extension hooks.

#### Conversations and agent tasks

Expose both ongoing conversation sessions and one-shot-style task execution through the same underlying agent loop. Conversations accept successive messages and retain complete session history. Tasks accept an explicit input, instructions, selected model/tools, and execution limits, then return an output or a failure/cancellation outcome. A task may involve multiple model turns and tool calls; one-shot refers to its bounded invocation, not a single model request.

Tasks should support declared output schemas and validation so consumers can reliably use their results in subsequent code or workflow steps. Each task has its own execution history and emits lifecycle events; task execution must not require an existing user conversation. Consumers choose its storage policy using the harness's storage interfaces.

#### Session state and storage contract

The harness defines and maintains session state, including the complete session history and all persisted state needed to reconstruct an idle session. Context selected for an individual model request is derived from that state; pruning or transforming model input must not discard session history.

The harness exports a `SessionStore` trait and accepts a consumer-provided implementation. Through that interface, it streams ordered persistence events for incremental saving and loads stored sessions. Consumers can also observe lifecycle events through the harness's event API. The consumer owns the storage backend and its implementation; the harness owns session types, persistence-event semantics, and reconstruction logic.

The required storage invariant is exact reconstruction at every idle boundary: given stored state `A` and subsequent persistence events `E1, E2, ..., En`, loading `A` plus those events must produce exactly the live session state `B` after the agent relinquishes control. Once those events are saved, the consumer must be able to drop the in-memory session and reload it without changing its state.

All state changes needed to reconstruct `B` must be represented in the persistence contract. An idle session is safe to discard only after the store has acknowledged all corresponding events; persistence failures must be surfaced rather than reporting the state as saved. This invariant concerns session state, not replaying external tool side effects or reproducing process-local handles and executors.

The harness must not depend on Metis's network contracts, native plugin loader, daemon configuration, or UI. It receives tools and hooks through Rust interfaces. Pi's `packages/agent` types, loop, state wrapper, and context transformation boundaries are useful references; do not copy its full application architecture by default.

### `workflow`

A reusable workflow definition and execution library, separate from `agent-harness`. It owns pipeline sequencing and data flow, while the harness owns model/tool execution inside an agent task.

- Define workflows as directed acyclic graphs (DAGs) in V1, with input/output schemas, named steps, and connectors.
- Support agentic tasks using `agent-harness`, runtime-provided Lua steps, and fan-in Lua steps.
- Pass validated step outputs into subsequent steps and expose progress, results, errors, and cancellation.
- Give agent steps narrowly scoped instructions, context, tools, model selection, and execution budgets instead of requiring a model to orchestrate the entire pipeline.
- Keep storage backends, scheduling, network transports, and concrete external integrations outside the crate; consumers supply these through interfaces or adapters.

#### Definition and routing

A workflow declares an entrypoint input schema, a final output schema, a list of named work steps, and connectors. The entrypoint and final output are implicit special named steps, not explicitly included in the work-step list; `$input` and `$output` are illustrative names. The list of steps does not determine execution order: dependencies and connector routing do.

Every unit that performs work is one of exactly three step types:

- **Agentic task:** a bounded model-driven task with explicit input and output.
- **Lua step:** runtime-provided Lua execution with explicitly declared tool dependencies, drawn from the enclosing agent context.
- **Fan-in Lua step (join):** a runtime-provided variation of the Lua step, with multiple declared input ports and no tool access.

Each connector references one source step, declares its possible destination steps/input ports, and contains a Lua routing function. The function receives the source step's output and returns a Lua table mapping selected destinations to their input values. It may omit declared destinations, allowing conditional routing and fan-out. It must not target undeclared destinations. The concrete representation of step/port keys remains open. Connector Lua transforms and routes data; tool access belongs to Lua work steps, not routing functions.

Each input port may be referenced by at most one connector destination, including across different connectors. Validate and reject multiple producers for the same port. Multiple distinct ports on a join allow fan-in. Mandatory-port connectivity requirements remain to be finalized.

#### Join readiness and unreachable branches

A join waits until every declared input port has either received a value or is confirmed unreachable for the current run. Connector omissions can abandon graph paths; the runtime propagates that reachability information through the DAG, accounting for pending upstream routing decisions. Do not mark a port unreachable merely because its value has not arrived yet.

Once all ports are resolved, execute the join's Lua function with received values and explicit unreachable markers, distinct from valid nil/null payloads. Route its result through its outgoing connectors as for other steps. Failed or cancelled branches are distinct from deliberately abandoned branches; how they affect join readiness remains open.

#### Declared tool dependencies

Each regular Lua step declares the tool functions it will use. Before execution, validate that each declared tool exists and is permitted by the workflow's agent context. Inject only those declared, permitted tool functions into the Lua function's inputs; do not expose the full tool registry or an unrestricted dynamic tool lookup. All external effects performed by Lua steps go through these tool calls, rather than direct filesystem, network, process, or native-library access through Lua globals or imports.

These declarations enable dependency and permission validation without parsing Lua's AST. They do not prove which calls the script actually makes: undeclared tool access is prevented by the restricted execution environment and rejected at runtime. Tool dispatch must also enforce the agent's permissions. Connectors and fan-in Lua steps receive no tool functions.

Conditional fan-out and explicit fan-in are V1 requirements; execution concurrency, workflow definition format, and durable checkpoint/resume behavior remain open. Workflow progress and outputs are distinct from an agent task's session history. Restoring harness session state does not by itself guarantee safe resumption of a workflow or prevent repeating external writes.

The daemon hosts workflow runs, supplies model/provider implementations and permission-filtered tool dispatch, and persists workflow state as needed. Plugins provide tools rather than custom workflow step types. Workflow Lua steps and agentic tasks invoke those tools through consumer-provided interfaces, without making `workflow` depend on the native plugin loader or requiring a registry of plugin-defined step executors.

Example: a voice recording passes through transcription, an agent task extracting structured intents, Lua validation and connector routing, narrowly scoped agent tasks where interpretation is still needed, and Lua steps calling tools to create calendar entries, todo items, and knowledgebase notes. A final step sends an ntfy notification summarizing actual results, including any partial failures. These integrations are illustrative capabilities, not mandatory crate dependencies.

This enables smaller local models to solve focused subproblems while explicit code controls the overall workflow. A stronger model could potentially perform the same work from instructions alone, but Metis must also support explicit pipelines without relying on that capability.

### `plugin-contract`

Defines the interface shared by native plugins and the plugin runtime:

- Manifest types and schema representation.
- Versioned C ABI definitions: function tables, opaque handles, buffers, status codes, request IDs, and host callbacks.
- Rust-facing plugin traits and adapter/export helpers that translate implementations into the C ABI.
- Request, response, initialization, tool capability, and event payload contracts as needed. Workflow integration uses the tool contract; plugins do not export workflow step implementations.

The Rust trait is internal to each compiled plugin. Rust trait objects, futures, strings, vectors, Tokio handles, and channels must not cross the C ABI boundary.

Keep dependencies small. SDK functionality can initially be feature-gated. Extract an optional `plugin-sdk` crate if export helpers, serialization, or async scheduling bring substantial dependencies; the runtime should only need the underlying contract.

### `plugin-runtime`

Owns daemon-side plugin management:

- Bundle extraction and manifest validation.
- Platform/architecture selection and shared-library loading.
- ABI and manifest compatibility checks.
- Plugin registry, initialization, configuration delivery, capability discovery, and dispatch.
- Receiving plugin events and completion callbacks.
- Request lifecycle, cancellation, shutdown, and shared-library lifetime management.

The daemon resolves configuration and secrets and supplies each plugin its own configuration plus an explicit host context. The runtime must not independently read the daemon's configuration files.

### `api-contracts`

Generated Rust network types and supported client/server stubs from `contracts/`.

Keep handwritten behavior out of generated code. Verify generator capabilities before assuming OpenAPI and AsyncAPI can generate complete interoperable client/server implementations.

Network DTOs are boundary types, not the internal data model for the harness, persistence, or plugin system. Convert them at application boundaries.

### `client`

Handwritten, UI-independent Rust daemon client built around generated contracts:

- HTTP calls, authentication, and endpoint configuration.
- WebSocket connections, subscriptions, event handling, reconnection, and resume behavior.
- Transport connectors for Linux/macOS Unix sockets, Windows named pipes, and remote TCP/TLS connections, sharing the same HTTP/WebSocket API.
- Cancellation and consistent application-facing error handling.

Used by `terminal` and the Tauri `app` on both desktop and Android. Avoid making it depend on Tauri or terminal UI libraries.

### `daemon`

The persistent application host and composition layer:

- HTTP and WebSocket API handlers.
- Local IPC listeners and an optional TCP/TLS listener feeding the same API handlers.
- Session/task orchestration, consumer-provided session storage implementing the harness's `SessionStore` trait, scheduling, and application policy.
- Configuration and secret resolution.
- Plugin initialization and adapters translating plugin capabilities into harness tools and hooks.
- Managing execution independently of connected UIs.
- Declaring agents and resolving their compact system prompts and permissions for every execution context.
- Hosting workflow runs, supplying permission-filtered tools, and managing their state, triggers, and results.

Begin with internal modules for storage, scheduling, authentication, and configuration. Extract crates when reuse or complexity justifies it rather than creating a broad generic `core` crate immediately.

### `terminal`

One crate containing CLI commands for scripting/automation and an interactive TUI. Both use `client` and the same daemon API. They do not implement a separate agent runtime.

### `app`

The Tauri shell for desktop and Android.

- Owns the shared Rust `client` and exposes application operations to SvelteKit through Tauri commands and events/channels.
- Provides platform integration such as credential storage, notifications, file selection, and desktop window/tray behavior as needed.
- Includes or integrates custom Kotlin Android components.

The app is a client of the persistent daemon. Closing the UI does not imply stopping daemon work. Do not embed daemon orchestration into the app shell by default.

### Optional `config`

Start as a daemon module. Extract if other crates require shared configuration behavior. Planned configuration sources include files, environment variables, CLI flags, and potentially keychains or native configuration stores. Source precedence and secret handling are not yet finalized.

## Native plugin design

A conceptual bundle could contain:

```text
example.metis
├── manifest.toml
├── lib/
│   ├── linux-x86_64/plugin.so
│   ├── macos-aarch64/plugin.dylib
│   └── windows-x86_64/plugin.dll
└── assets/
```

Manifest format, extension, supported target names, and single-target versus multi-target packaging remain open. Plugins need matching OS/architecture builds. Other languages can implement the C ABI directly; Rust authors use the SDK's trait and export adapters.

Proposed ABI direction:

- One versioned exported entry point returning a function table.
- Explicit instance creation and destruction.
- Capability discovery and initialization with plugin-specific configuration.
- Serialized request/response payloads, initially considering JSON bytes.
- Explicit ownership and release functions for plugin-allocated buffers.
- Host callbacks for completion, progress, and plugin-originated events.

Distinguish bundle/manifest format version, ABI version, and plugin version. All ABI operations need defined ownership, concurrency, and error semantics. Native plugins execute in the daemon process and can crash it; a C ABI is not a sandbox.

### Async boundary

The harness and daemon are expected to use async Rust. Do not pass Rust futures across the ABI.

The discussed direction is a start/completion protocol:

1. Host registers a request ID and completion channel.
2. Host calls the plugin's start function.
3. Plugin schedules work and returns promptly.
4. Plugin invokes a host completion callback with the request ID and result.
5. Host resolves the awaiting Rust task.

Plugin-owned executors hidden behind the Rust SDK are the initial preferred approach, not a finalized implementation. Completion may occur before start returns, so pending state must be registered first. Cancellation is an explicit request; acknowledgement and completion races need defined semantics.

Contain Rust panics within exported adapters where possible. Define callback thread-safety and shutdown ordering. Keep the library loaded until all instances, tasks, callbacks, and plugin-owned buffers are finished. Defer hot unloading until these guarantees are implemented; disabling a capability does not require immediately unloading its library.

## Network contracts

`contracts/` is authoritative:

- `openapi.yaml`: ordinary HTTP endpoints.
- `asyncapi.yaml`: WebSocket channels, message schemas, and communication semantics.
- `schemas/`: shared payload schemas where supported by the chosen tooling.

Define realtime semantics explicitly, including correlation/request IDs, event IDs or cursors, protocol versioning, subscriptions, cancellation, ordering, reconnection/resume, expired cursors, and slow consumers. Schema specifications alone do not implement these behaviors.

Generated TypeScript boundary types may be useful to the frontend, but the planned packaged apps reuse the Rust network client. A handwritten TypeScript network client is not required unless a standalone browser client is introduced.

### Local and remote transports

Retain HTTP, WebSockets, OpenAPI, and AsyncAPI for both local and remote communication. HTTP does not require TCP: local connections carry the same HTTP requests and WebSocket upgrade/frame protocol over platform-specific bidirectional streams.

| Platform | Required local transport |
|---|---|
| Linux | Unix domain socket |
| macOS/Darwin | Unix domain socket |
| Windows | Duplex, byte-mode named pipe |

The daemon and Rust client must support these transports through listener/connector adapters, without separate endpoint implementations or message schemas. Generated clients may require custom connectors. Document local endpoint discovery and configuration alongside the contracts. A cross-platform library such as `interprocess` is an implementation candidate, not a selected dependency.

Local IPC is the default desktop connection mode. Enable network listening explicitly for remote/device access. Restrict Unix socket directories and socket access to intended users. Configure Windows pipe security descriptors for intended users and reject remote pipe clients; named pipes must be made explicitly local-only.

Tauri desktop clients access local IPC through the shared Rust client. Standalone browser JavaScript cannot directly connect to Unix sockets or Windows named pipes and requires a network HTTP/WebSocket endpoint or gateway. Android remote access uses the network transport; local Android daemon hosting is not required by this desktop IPC decision.

## UI and Android integration

Shared application path:

```text
SvelteKit UI
    ↕ Tauri commands and events/channels
Rust client
    ↕ HTTP and WebSocket
Metis daemon
```

Keep backend/session operations separate from host-system capabilities. Expose platform capabilities explicitly so screens can adapt to Android versus desktop features. Batch high-frequency streamed updates where appropriate instead of crossing the bridge for every token.

Android system assistant integration requires native components and manifest declarations. An `ACTION_ASSIST` activity can support assistant activation; fuller voice interaction uses `VoiceInteractionService` and associated sessions. The user must select Metis as assistant, and invocation gestures vary by device/configuration.

Keep the assistant service lifecycle independent of the WebView. Do not assume Tauri or the frontend is initialized when Android starts a service. Design cold-start activation, service-to-client communication, permissions, background behavior, and resume/reconnection explicitly. Tauri does not exempt the client from Android lifecycle restrictions.

## Dependency direction

```text
daemon → agent-harness
daemon → workflow → agent-harness
daemon → plugin-runtime → plugin-contract
daemon → api-contracts

terminal → client → api-contracts
app      → client → api-contracts
ui       → Tauri bridge → app

external Rust plugin → plugin-contract SDK feature
                      (or optional plugin-sdk → plugin-contract)
```

Plugin-to-harness adapters initially belong in the daemon. The plugin contract and reusable harness should evolve independently.

## Open decisions and early validation

- Exact Rust interfaces, C ABI layout, payload schemas, and async SDK executor model.
- Bundle format and platform support matrix.
- OpenAPI/AsyncAPI generators, generated-code placement, and regeneration checks.
- Configuration precedence and secret distribution.
- Storage, model providers, terminal UI library, and scheduling details.
- Exact realtime replay/resume and cancellation semantics.
- Agent-task output contracts and workflow definition format, failure/retry policy, and optional durable checkpoint/resume semantics. Retries of external writes need explicit idempotency or reconciliation rather than an assumed exactly-once guarantee.
- Workflow step/port addressing, mandatory-port connectivity, concurrency, and failed/cancelled branch handling at joins. Final naming and declaration format for agents, permission categories, and Lua tool-injection API/runtime restrictions.

Validate HTTP calls and WebSocket upgrades over Linux/macOS Unix sockets and Windows named pipes early, including endpoint access controls and reconnection. Validate Android assistant activation from a cold start, Tauri/SvelteKit keyboard and scrolling behavior, shared Rust-client reconnection after background/resume, and a release Android build early. Validate plugin completion/cancellation races and library lifetime guarantees before enabling runtime unloading.

Implement a small coherent system first. Keep the harness, plugin ABI, network contracts, and client independent enough to evolve without forcing changes across every layer.
