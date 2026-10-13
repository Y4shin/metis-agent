---
type: changelog
title: Task Changelog
---

# Task Changelog

## 2026-10-13, Pairing over the remote listener (remote-pairing)

Pairing is now reachable from a device with no local socket: a client pairs over the TCP listener with the preshared key, keeps the minted token, and serves authenticated calls on the same transport. Pairing became optional, since an absent or empty `METIS_PRESHARED_KEY` resolves to `None` and closes pairing while every other call keeps serving, and an unresolvable reference stays a loud error. All three refusal causes, disabled, wrong key and already paired, now travel in a `metis-pair-error` header and reach the client as typed errors instead of `PermissionDenied` or `AlreadyExists`. Closing pairing is a state rather than a revocation, so surfaces paired earlier keep serving until their token row is deleted; the missing rate limit on network-reachable pairing attempts is recorded as fog.

## 2026-10-13, Service-taking TCP front-end (serve-tcp-service)

`transport::serve_tcp_service` and `serve_tcp_listener_service` now mount a `ConnectRpcService` on TCP, the way `serve_uds_service` already did on the Unix socket, because `connectrpc` keeps the interceptor chain on the service rather than the router. Both transports can therefore enforce the credential policy the last ticket added, with no peer-address sniffing: the tests refuse a credentialless call with the typed missing-credential error, serve a paired token, keep `Pair` exempt, and assert the two front-ends answer identically. `serve_tcp_listener` and `serve_stream` were reduced to delegations over the one private connection body, keeping their signatures and observable behaviour, and the workspace suite is at 144 passed, 0 failed.

## 2026-10-13, Pairing, per-surface tokens, and client credential storage (pairing-and-tokens)

`DaemonService` gained `Pair`, gated by a preshared key resolved from a literal, an environment reference, a keystore or a secret file, with a group- or world-readable secret file refused rather than warned about. A successful pairing mints one opaque per-surface token, stores only its SHA-256 in the state database, and returns the text once; the daemon authorizes unary and streaming calls through a `connectrpc::Interceptor` that reads `Authorization: Bearer` and answers `metis-auth-error` with missing, invalid or revoked, while the client keeps its token in the platform keychain through `secret-tool` or `security`, or in a 600 file, and reports which answered. Removing a token row and restarting revokes that surface alone. A listener mounts a `ConnectRpcService` rather than a `Router`, so `serve_uds_service` carries the policy on the Unix socket and the TCP equivalent is the new `serve-tcp-service` ticket.

## 2026-10-13, Daemon state store on SQLite (state-store)

`crates/daemon` now has a library target, and `daemon::state::Store` opens and migrates one SQLite database through sqlx runtime queries: `METIS_STATE_DB` first, then `$XDG_DATA_HOME/metis/metis.db`, then `$HOME/.local/share/metis/metis.db`, with the directory at mode 700 and the file at 600 through the same private-path rule the socket uses. The `tokens` table holds a surface name, the 32-byte SHA-256 of the token's canonical lowercase hex text, and the pairing time, so per-surface revocation is a row delete and the token itself is never stored; `daemon::token` mints and parses it, with the digest pinned by a known-answer test. CI needs no database server, no `DATABASE_URL` and no `.sqlx` offline data, because the store uses runtime queries and embeds its migration.

## 2026-10-13, Local access control and surface identity (local-identity-and-acl)

The daemon now creates its socket directory and the socket itself with mode 700, or 770 when `METIS_SOCKET_GROUP` names a group its user belongs to, and refuses a pre-existing directory that is a symlink, is owned by someone else, or is wider than the rule, instead of chmodding it. Every client call declares `metis-protocol-version`, `metis-surface` and `metis-client-version` as Connect default headers, the service records them, and a missing, malformed or unsupported version is refused with `InvalidArgument` carrying `metis-supported-versions`, which the wrapper turns into `ClientError::ProtocolVersion`; the declaration is not authentication, and credentials arrive with `pairing-and-tokens`. The default endpoint moved off `/tmp/metis-daemon.sock` to `$XDG_RUNTIME_DIR/metis/daemon.sock`, then `$TMPDIR/metis/daemon.sock`, then a typed `NoPrivateBase` error, which closes the conflict between the old `/tmp` default and the ownership rule.

## 2026-10-13, Remote TCP listener, off by default (tcp-listener)

The daemon's service is now reachable over TCP when `METIS_TCP_ADDR` names an `IP:port` literal, and no socket is opened when it does not: there is no default bind address, an absent or empty value disables the listener, and a malformed value is a typed `ConfigError` rather than a silent disable. `transport::serve_tcp_listener` is the one TCP accept loop over the landed `serve_stream`, `serve_tcp` is the bind-and-delegate entry point the daemon will call, `remote_authority` keeps the connection authority and the request authority in step, and `Client::with_remote_addr` reaches the listener without adding a transport injection point to the wrapper. The listener is plaintext and unauthenticated until the TLS, proxy and pairing tickets land, and `serve_tcp` itself has no direct test yet, which the operator accepted as a documented gap.

## 2026-10-13, Plugin version tolerance across compiled versions (plugin-compatibility)

`metis_read_table` now compares `abi_minor` additively with `found >= known`, so a plugin built for a newer minor inside major 1 interoperates while the retired pre-release table at minor 0 is still refused with `MinorMismatch` before any entry is read. A library built for another major fails at symbol resolution with `MajorMismatch` over a real second C library, and the fixture advertises an eighth capability at descriptor version 2, which makes the per-capability gate falsifiable end to end: only that capability is refused and the rest of the plugin keeps serving. Payload types prove that a body carrying an unknown field still decodes, and the harness grew from 62 to 73 checks, including Linux-only faulting-page probes that prove a reader touches only the entries its own `size` covers.

## 2026-10-13, Unify the capability declaration (plugin-contract-v1, amendment)

The declaration both sides of the plugin wire read is now one ungated type in `plugin-contract::capability`, with `serde_json::Value` schemas and a lenient `kind`; `schemars` stays optional behind `authoring`, and the host deleted its own duplicate type and the test that kept the two in step. A capability whose kind this build does not know is refused on its own with `Unsupported` instead of failing the declaration. The wire is byte-identical.

## 2026-10-13, Plugin contract v1: futures-only ABI (plugin-contract-v1)

Futures are now the only way the daemon calls a plugin: `invoke` and the `complete` callback are deleted, `invoke_future`, `service_complete` and `service_request` are the table, and the authoring surface is one capability-name-routed `call` instead of four role-named arms, which removes the four-capability ceiling. The plugin owns no executor and no thread, `async-ffi` is dropped because a destructor panic inside a plugin future aborts the process even for a plugin that returned `Ok`, and unload is quarantine plus one reclaim on a late poll return. `plugin-runtime` gained the loader, the registry, both-direction schema validation and the unload ledger, and a hand-written 482-line C plugin compiles under gcc and clang and is driven end to end by the harness.

## 2026-10-13, Plugin authoring trait and adapters (plugin-authoring-trait)

Plugin authors now write ordinary async Rust: `plugin-contract` gained the feature-gated `authoring` module with the `Plugin` trait, an awaitable cancellation token on the per-request context, the capability declaration, a `const fn` function table, a host adapter, and a minimal executor, while the runtime still depends on the primitives and payload envelope alone. The adapter always completes through the host callback (the ABI keeps its inline path for direct implementers), the trait module names no ABI type, lifecycle hooks and plugin-originated events have routes, and the fixture was rewritten through the trait with the same capability names and single exported symbol.


## 2026-10-13, Local transport and the echo RPC skeleton (local-transport-rpc-skeleton)

Made the first real path between two processes: one handwritten transport module in `api-contracts` that serves any Connect service over a Unix domain socket (and over any async bidirectional stream), plus a UI-independent `client` wrapper that calls the echo RPC without the caller naming a transport, and a runnable example proving it end to end. The endpoint is a platform default with a `METIS_ENDPOINT` override, and the socket tests clean up after themselves. Nothing in the generated trees changed.


## 2026-10-13, Contract gates read one configuration and authenticate to the BSR (contract-gate-hardening)

Moved the buf configuration to the repository root, so `buf lint`, `buf breaking`, and generation read one `buf.yaml` and the declared policy is the one that runs; `--against-config buf.yaml` keeps both sides of the breaking comparison scoped to `contracts/proto`. CI supplies `BUF_TOKEN` from `secrets.BUF_TOKEN` to the single plugin-resolving step and unsets it when the secret is empty, and the gate configuration gained an on-demand acceptance probe at `scripts/probe-contract-gates.sh`. Measured: anonymous resolution is rate limited, an unset or empty token reaches the BSR anonymously, and a non-empty unverifiable token is refused before any request.


## 2026-10-13, Plugin ABI primitives proven by a fixture plugin (plugin-abi-core)

Made the plugin boundary real: `crates/plugin-contract` now holds the `repr(C)` primitive ABI (versioned entry symbol, append-only function table with `size` first, host callback table with `user_data`, status space with `pending`, payload envelope), with the layout asserted in Rust and by a C translation unit compiled on every build. A committed cbindgen header with a drift check, plus a fixture `cdylib` and a 41-check harness that loads it and drives the whole table, including per-side buffer release and panic containment on both sides.


## 2026-10-13, Contract pipeline with one RPC (contract-pipeline)

Made the wire contract real: hand-written Protobuf under package `metis.v1` with
one `Echo` RPC on `DaemonService`, generated by the two pinned BSR remote plugins
into committed Rust in `crates/api-contracts`, mounted with `include!` and the two
measured clippy allows. One regeneration command with a byte-for-byte drift check,
plus `buf lint` and `buf breaking` in CI and no JVM, `protoc`, or local plugin
binary. The contract gates and the generated trees now cover the whole workspace
through the existing fmt, clippy, and test steps.


## 2026-10-12, Scaffold the Metis repository skeleton (scaffold-repo-skeleton)

Added the nine-crate Cargo workspace with the agreed dependency direction, the `contracts/` tree, the `apps/ui` SvelteKit project, and the shared tooling (`rustfmt.toml`, `.editorconfig`, GitHub Actions CI, AGPL-3.0 `LICENSE`, Node.js and npm in devenv). No product code was written. `cargo check`, `cargo fmt --check`, `cargo clippy`, `cargo test`, and the frontend checks all pass.

## 2026-10-12, Create the five followup effort maps (create-followup-efforts)

Charted `metis-foundations`, `metis-agent-core`, `metis-plugins`, `metis-surfaces`, and `metis-plugin-sdk` as low-resolution Wayfinder maps with effort dependency edges, and registered them in the task index. Each followup charts its own frontier in a later Wayfinder session.
