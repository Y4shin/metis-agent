---
type: map
title: Metis surfaces
status: stable
blocked_by: [metis-foundations]
---

# Metis surfaces

## Destination

Metis has user-facing shells. A Tauri 2 desktop application and an Android
application share one SvelteKit frontend, statically built for Tauri, and both
reach the daemon only through the shared Rust `client` crate over Connect. The
desktop shell owns the Rust client and exposes application
operations to SvelteKit through Tauri commands and events or channels. On
Android, the shell integrates with the system assistant: an `ACTION_ASSIST`
activity for assistant activation and, where needed, `VoiceInteractionService`
and its sessions. Platform capabilities such as credential storage,
notifications, file selection, and window or tray behaviour are explicit, so
screens can adapt between desktop and Android. The UI reflects daemon state
without owning it: closing the UI does not stop daemon work.

## Constraints

- Backend functionality belongs in Rust, not in SvelteKit server routes.
- The frontend is statically built for Tauri.
- The `app` crate owns the shared Rust `client`; the frontend talks to the
  daemon only through the app's commands and events.
- The application is a client of the persistent daemon. Do not embed daemon
  orchestration in the app shell by default.
- High-frequency streamed updates are batched across the Tauri bridge rather
  than crossing it per token.
- The Android assistant service lifecycle is independent of the WebView. Do
  not assume Tauri or the frontend is initialised when Android starts a
  service.
- Cold-start activation, service-to-client communication, permissions,
  background behaviour, and resume and reconnection are designed explicitly.
  Android lifecycle restrictions apply regardless of Tauri.
- Android remote access uses the network transport. Local Android daemon
  hosting is not required.
- No iOS implementation.

## Decisions so far

- From `docs/rough-plan.md`, "Agreed technology choices": Tauri 2 for desktop
  and Android, SvelteKit for the shared frontend, custom Kotlin or Java
  Android integration where needed.
- From "UI and Android integration": the SvelteKit to Tauri to Rust client to
  daemon path, separation of backend and session operations from host-system
  capabilities, explicit platform capabilities, batched streaming, and the
  Android assistant design notes.
- From "Local and remote transports": Tauri desktop clients use local IPC
  through the shared Rust client; Android uses the network transport; a
  standalone browser client would need a network endpoint or gateway.

## Fog

- Android system assistant integration details: which activation path is
  sufficient first, and when `VoiceInteractionService` is required.
- Cold-start activation, permission requests, background behaviour, and the
  service-to-client protocol.
- Shared Rust-client reconnection after background and resume.
- Release Android build: signing, packaging, and CI.
- Credential storage, notifications, and file selection on each platform.
- Tauri and SvelteKit keyboard and scrolling behaviour, and desktop window and
  tray behaviour.
- Which screens exist first and how daemon state is projected into them.
- Whether a standalone browser client is ever needed, which would require a
  network endpoint or gateway.

## Out of scope

- The network contract, generated types, client, and the realtime design are
  owned by `metis-foundations`. It proves the transports end to end with one
  dummy request and one dummy event. The real endpoints are implemented in
  `metis-daemon`, not here.
- The daemon, its IPC listeners, and its policy layer.
- The Rust `client` crate, the network contracts, and the generated types,
  which belong to `metis-foundations`.
- The agent harness, workflows, and plugins.
- Any backend logic in the frontend.
