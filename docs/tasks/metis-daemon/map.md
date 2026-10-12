---
type: map
title: Metis daemon
status: stable
blocked_by: [metis-agent-core, metis-foundations, metis-plugins]
---

# Metis daemon

## Destination

The persistent daemon is a real process and a real composition layer.
`crates/daemon` serves the contract API over the local IPC transport (Unix
domain socket on Linux and macOS, duplex named pipe on Windows) and over an
explicitly enabled TCP/TLS listener, from one endpoint implementation shared by
both. It hosts sessions and tasks on `agent-harness` with its own
`SessionStore` implementation, owns scheduling, resolves configuration and
secrets, declares agents and resolves their prompts and permissions for every
execution context, adapts plugin capabilities into harness tools, providers, and hooks,
implements the model providers the harness consumes, hosts workflow runs with
permission-filtered tool dispatch, and keeps running independently of any
connected UI. `crates/terminal` provides the CLI commands and the interactive
TUI against the daemon through `client`, so the daemon can be exercised end to
end and work can land as vertical slices rather than as an untested layer.

## Constraints

- The daemon depends on `agent-harness`, `workflow`, `plugin-runtime`, and
  `api-contracts`. It does not depend on `client`, `terminal`, or the app
  shells.
- One endpoint implementation serves the local IPC listener and the optional
  TCP/TLS listener. No duplicate handlers and no second schema.
- Configuration and secret resolution live here. The plugin runtime never
  reads daemon configuration; it is given each plugin its own configuration
  plus an explicit host context.
- Concrete model provider implementations live here, not in `agent-harness`.
- The daemon owns session state policy through a `SessionStore`
  implementation. Persistence failures are surfaced, never reported as saved.
- Execution continues independently of connected UIs. Closing a UI does not
  stop daemon work.
- Every conversation, task, and workflow run belongs to a declared agent
  context. A task or step may narrow the enclosing permissions but never widen
  them.
- Workflow runs are hosted here, with tools dispatched under the run's agent
  permissions. A surface does not host workflow execution.
- Native plugins run in the daemon process and can crash it. The containment
  policy is explicit, and plugin installation is not a daemon-internal concern.
- `terminal` uses `client` only. The CLI and the TUI share one API and
  implement no agent runtime.
- The optional `config` module starts here as a module. Extract a crate only
  when reuse or complexity justifies it.

## Decisions so far

- From `docs/rough-plan.md`, "daemon": HTTP and WebSocket API handlers, local
  IPC listeners and an optional TCP/TLS listener feeding the same handlers,
  session and task orchestration with storage implementing the harness's
  `SessionStore` trait, scheduling and application policy, configuration and
  secret resolution, plugin initialisation and adapters, execution independent
  of connected UIs, agent declarations and prompt/permission resolution, and
  workflow hosting with permission-filtered tools.
- From "terminal": one crate holding CLI commands and an interactive TUI, both
  using `client`, neither implementing an agent runtime.
- From "Dependency direction": `daemon → agent-harness`,
  `daemon → workflow → agent-harness`, `daemon → plugin-runtime →
  plugin-contract`, `daemon → api-contracts`, and
  `terminal → client → api-contracts`.
- This effort owns `crates/daemon` and `crates/terminal`. The terminal lives
  here rather than in `metis-surfaces` because it is the tool that exercises
  the daemon end to end and lets work land as vertical slices.
  `metis-surfaces` stays Tauri and Android only.
- The real endpoints named in the other efforts' cross-effort notes are
  implemented here. `metis-foundations` ships only a thin skeleton endpoint to
  prove the transports.
- The transports are owned by `metis-foundations`: Unix domain sockets and
  named pipes for local IPC, and an explicitly enabled TCP/TLS listener. This
  effort implements the listener side against that contract.
- Local IPC is the default. Remote access stays off until explicitly enabled.
- Decided during the `metis-foundations` grilling session. Configuration is read
  once at startup, and changing it requires a restart. A secret configuration
  value is a literal, an environment variable reference (`$ENV:NAME`), or an OS
  keystore reference (`$KEYSTORE`) resolved to the keystore item whose canonical
  name is the config item's path, or a secret file reference
  (`$SECRET_FILE:path`, relative to the config file) whose permissions must grant
  the owner read access and grant group and others nothing (400, 500, 600, or
  700); a secret file with looser permissions is rejected. This applies to every
  secret, including listener keys, provider credentials, and plugin
  configuration.

## Fog

- Authentication between surfaces and the daemon: scheme, credential issuance
  and storage, per-surface scopes, and what local IPC trust means. The
  boundary between the client's credential transport and the daemon's policy
  is not yet assigned to an effort.
- Configuration precedence across files, environment variables, CLI flags, and
  keychain or native stores, and how secrets reach plugins.
- Which storage backend implements `SessionStore`, and where that code lives.
- Concrete model providers: which come first, how models are selected, and how
  their credentials are stored.
- Agent declarations: the declaration format, prompt and permission
  resolution, and the permission categories, shared with `metis-agent-core`.
- Session and task orchestration: lifecycle, scheduling and concurrency,
  cancellation propagation, and what a disconnect does to running work.
- Workflow hosting: triggers, persisted run state, progress and result delivery
  to clients, and failed or cancelled run handling.
- Plugin adapters: mapping plugin capabilities to harness tools, providers, and
  hooks, per-plugin configuration delivery, and crash containment policy.
- Terminal: the TUI library, the CLI command surface, and the first vertical
  slice the terminal drives.

## Out of scope

- The contracts, the generated types, and the `client` crate, owned by
  `metis-foundations`. This effort implements the server side of that
  contract.
- The `agent-harness` and `workflow` crates, owned by `metis-agent-core`. This
  effort consumes their interfaces.
- The plugin host (bundle extraction, loading, compatibility checks, registry,
  dispatch, lifetime management), owned by `metis-plugins`, and the authoring
  SDK, core plugins, and plugin CLI, owned by `metis-plugin-sdk`.
- The Tauri shell, the Android shell, and `apps/ui`, owned by
  `metis-surfaces`.
- Provider plugins and plugin-defined capabilities. They arrive through the
  plugin runtime, not as daemon code.
- Local Android daemon hosting. Android reaches the daemon over the network
  transport.
