---
type: ticket
subtype: feature
title: Local transport and the echo RPC skeleton
status: stable
workflow_state: ready
blocked_by: [contract-pipeline]
size: m
---

## What to build

The first complete path between two processes: shared Unix-socket listener and
connector glue, a client wrapper that hides the transport, and a minimal service,
so that a terminal-side example calls the daemon's echo RPC over the local socket
and prints the answer.

## Acceptance criteria

- [ ] A shared transport module can serve a Connect service over a Unix domain
      socket and connect a client to one, working over any asynchronous
      bidirectional stream.
- [ ] The client wrapper performs the echo RPC over the local transport without
      the caller naming a transport.
- [ ] A test starts the service on a temporary socket path and asserts the echoed
      response, and the test cleans up the socket directory.
- [ ] An example binary performs the same call and prints the response.
- [ ] The endpoint comes from the platform default, with an environment variable
      override honoured.
- [ ] The client stays free of Tauri and terminal UI dependencies.

## Blocked by

- `contract-pipeline`, because there is no service to call until the contract and
  its generated code exist.

## Implementation notes

Landed on `task/local-transport-rpc-skeleton` as the merge commit `d886567`
("ticket(local-transport-rpc-skeleton): Local transport and the echo RPC
skeleton"), parents `c4daca7` (the landing branch, which was level with `main`)
and `3db4066` (the working branch `ticket/local-transport-rpc-skeleton`). The
merge was clean with no conflicts, the landing tree is byte-identical to
`3db4066` (`git diff 3db4066 HEAD` is empty), and
`ticket/local-transport-rpc-skeleton` was deleted after the merge. Because the
landed tree is the tree the verify worker tested, no gate was re-run at landing.
The review artifact `deviation-reports/local-transport-rpc-skeleton.md` is
committed with this note.

1. The transport seam lives in `crates/api-contracts/src/transport.rs`, mounted
   as `pub mod transport`. `serve_stream<S>` is generic over any
   `AsyncRead + AsyncWrite + Unpin + Send` stream, wraps it with `TokioIo`,
   builds `ConnectRpcService::from_arc`, and drives a `hyper_util` auto (HTTP/1.1
   or HTTP/2) connection; `serve_uds` and `uds_transport` are the listener and
   connector front-ends over it, and `local_authority()` is `http://localhost`.
2. `serve_stream` takes `router: impl Into<Arc<Router>>` rather than the arch
   spec's `router: Router`, because `connectrpc::Router` is not `Clone` in 0.9.1
   and an accept loop cannot hand one router to every accepted stream. A bare
   `Router` still compiles, which is the shape the duplex test uses; `serve_uds`
   keeps `router: Router` and wraps it in an `Arc` itself. `windows-named-pipe`,
   `tcp-listener`, `tls-and-client-trust` and `reverse-proxy-mode` must build the
   router once and clone the `Arc` per accepted stream, and the arch spec
   sentence naming `ConnectRpcService::new` is stale.
3. The wrapper in `crates/client/src/lib.rs` holds the generated client over the
   injected transport and exposes `local()`, `with_endpoint(..)`, and
   `echo(&str) -> Result<String, ClientError>`. The caller names no transport, no
   `Http2Connection`, and no generated stub type; `ClientError` is a
   `#[non_exhaustive]` enum with `Call(ConnectError)`, `Display`, `Error`, and
   `From<ConnectError>`. The transport field is private, so every new transport
   is a `client` change rather than an injection from outside.
4. The two endpoint literals are fixed here: the platform default
   `/tmp/metis-daemon.sock` and the override variable `METIS_ENDPOINT`, with an
   empty value treated as unset. `resolve_endpoint()` is additive beyond the arch
   spec's export list and is what makes the wrapper and the example honour the
   override identically; `const ENDPOINT_ENV` is private, so the daemon effort
   must restate the variable name unless it is exported.
5. `crates/client/src/skeleton.rs` is a public module documented as test support:
   it is the one implementation the integration test and the example drive, and
   it is the reason `tokio` is a normal dependency of `client`. Whether to
   feature-gate it until the daemon effort removes it is open, and not blocking.
6. The UDS front-ends and `Client::local` / `Client::with_endpoint` are
   `#[cfg(unix)]` for now; the Windows named-pipe default arrives with
   `windows-named-pipe`. `crates/client/examples/echo.rs` serves the skeleton on
   the resolved endpoint, calls echo through the wrapper, prints `[echo]` and
   `[result] PASS`, and exits non-zero on a bad endpoint. It pins
   `#[tokio::main(flavor = "current_thread")]`, since `rt-multi-thread` is
   deliberately absent from the narrowed workspace tokio features.
7. Manifest work: `tokio`, `tower`, `hyper`, `hyper-util` and the pinned
   `connectrpc` are declared once in the root `[workspace.dependencies]`; the
   `connectrpc` pin and features are unchanged, so the effective dependency graph
   is unchanged. `docs/testing.md` and `docs/dev-env.md` carry the local-transport
   run commands.
8. The ticket doc has no test-plan section, so the acceptance test is
   `crates/client/tests/echo.rs` (4 tests: the duplex loop seam, the served path
   through the wrapper on a temporary socket directory with a `Drop` cleanup
   guard, the platform default literal, and the environment override plus
   `Client::local()`), run by the existing `cargo test` step. `cargo fmt --check`,
   `cargo clippy --all-targets -- -D warnings`, the client suite and the full
   workspace suite were recorded green by the TDD and verify workers on this
   exact tree.
9. Residual risks: `serve_uds` has no shutdown path that removes its socket file,
   so the example leaves one file behind on its endpoint; the override test
   mutates the process environment with `unsafe { std::env::set_var }` (edition
   2024), safe only while it is the sole reader of `METIS_ENDPOINT` in that test
   binary; and `ClientError::Display` hardcodes "echo call failed", fine while
   echo is the only RPC.
10. Open item for the effort owner, not blocking this ticket's acceptance
    criteria: the `/tmp` default conflicts with `local-identity-and-acl`'s
    requirement that the socket and its directory be created with a configured
    owner, group and mode 700 or 770, since `/tmp` is world-writable and sticky
    and `serve_uds` unlinks whatever sits at the path before binding. Decide
    between a daemon-owned runtime directory (for example
    `$XDG_RUNTIME_DIR/metis/daemon.sock` with a documented fallback) and moving
    the directory half of that requirement into the ACL ticket. Both
    "User attention needed" items are in the deviation report.
