---
type: map
title: Metis foundations
status: stable
blocked_by: []
---

# Metis foundations

## Destination

The network and plugin interfaces are real. `contracts/` is authoritative and
holds an OpenAPI document for ordinary HTTP, an AsyncAPI document for
WebSocket realtime communication, and shared payload schemas. The plugin
contract crate defines the manifest and the versioned C ABI. The
`api-contracts` crate carries generated Rust network types, and the `client`
crate speaks HTTP and WebSocket to the daemon over Unix domain sockets on
Linux and macOS and duplex byte-mode named pipes on Windows, with the same
API reachable over an explicitly enabled TCP/TLS listener. A thin end-to-end
skeleton runs: the terminal asks the daemon something over the local IPC
transport and prints the response, proving the transport and realtime
assumptions before anything is built on top.

## Constraints

- `contracts/` is the single source of truth for network types. Generated code
  stays generated, and handwritten behaviour does not live inside it.
- Network DTOs are boundary types, not the internal model of the harness,
  persistence, or plugin system. Convert at application boundaries.
- The `client` crate stays UI-independent: no Tauri, no terminal UI library.
- Local IPC is the default desktop transport. The TCP/TLS listener is off
  unless explicitly enabled.
- Unix socket directories and sockets are restricted to the intended users.
  Windows pipes are local-only with security descriptors for the intended
  users and remote pipe clients rejected.
- The detailed realtime semantics are defined before they are implemented:
  correlation and request IDs, event IDs or cursors, protocol versioning,
  subscriptions, cancellation, ordering, reconnection and resume, expired
  cursors, and slow consumers.
- Rust only. No TypeScript network client unless a standalone browser client
  is introduced.

## Decisions so far

- From `docs/rough-plan.md`, "Network contracts" and "Local and remote
  transports": OpenAPI for HTTP, AsyncAPI for WebSocket, `contracts/`
  authoritative, one endpoint implementation shared across local and remote
  transports.
- From "api-contracts": verify generator capabilities before assuming OpenAPI
  and AsyncAPI can generate complete interoperable client and server
  implementations.
- From "client": handwritten, UI-independent, sharing one HTTP and WebSocket
  API across the three transports.
- From "plugin-contract": manifest types, the versioned C ABI, and the
  Rust-facing trait and export adapters; small dependencies; SDK functionality
  feature-gated or extracted into `metis-plugin-sdk`.
- The walk is a walking skeleton first: prove the transports and the contract
  tooling end to end before widening either.

## Fog

- Which OpenAPI and AsyncAPI generators are usable, where generated code is
  placed, and how regeneration is checked in CI.
- Whether the plugin C ABI structs are laid out first in `repr(C)` Rust, in a
  C header, or in a neutral schema that both derive from.
- Local endpoint discovery and configuration: where the socket or pipe name
  comes from, and how the client finds it.
- Windows pipe security descriptor specifics and the exact rejection path for
  remote clients.
- Certificate and key handling for the optional TCP/TLS listener.
- How much of the realtime surface the walking skeleton exercises before the
  full semantics land.

## Out of scope

- Provider implementations.
- Full daemon behaviour beyond the thin skeleton endpoint.
- The plugin loader and runtime host, which belong to `metis-plugins`.
- The plugin authoring SDK, which belongs to `metis-plugin-sdk`.
- Agent loop, sessions, and workflows, which belong to `metis-agent-core`.
- Any UI or application shell.
