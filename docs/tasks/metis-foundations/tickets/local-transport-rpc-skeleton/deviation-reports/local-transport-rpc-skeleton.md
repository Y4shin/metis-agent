---
type: deviation report
title: Deviation report for local-transport-rpc-skeleton
status: stable
---

## Deviation report: local-transport-rpc-skeleton

Reviewed: `task/local-transport-rpc-skeleton..ticket/local-transport-rpc-skeleton`
(12 files, +670/-9, five checkpoint commits), plus the source files themselves.
The landing branch exists and is `main` (`c4daca7`), so the working branch was
diffed against it directly.

### API surface changes

**The transport exports match the plan except for the router parameter.**
- **Planned:** `pub async fn serve_stream<S>(stream: S, router: Router) ->
  io::Result<()>`, building the service with `ConnectRpcService::new(router)`,
  plus `serve_uds(path, router)`, `uds_transport(path)`, and
  `local_authority()`, mounted as `pub mod transport` in `api-contracts`.
- **Actual:** `pub async fn serve_stream<S>(stream: S, router: impl
  Into<Arc<Router>>) -> io::Result<()>`, building with
  `ConnectRpcService::from_arc(router.into())`. `connectrpc::Router` is not
  `Clone` in 0.9.1, so an accept loop cannot hand each accepted stream the same
  router; `serve_uds` keeps `router: Router` and wraps it in an `Arc` itself. A
  caller passing a bare `Router` still compiles, which is the shape the duplex
  test uses.
- **Impact:** on dependents, medium. `windows-named-pipe`, `tcp-listener`,
  `tls-and-client-trust`, and `reverse-proxy-mode` must build the router once and
  clone the `Arc` per accepted stream, and the arch spec's sentence naming
  `ConnectRpcService::new` is stale. This is the only planned export whose
  signature moved.

**Two additive public functions beyond the export list.**
- **Planned:** `default_endpoint()` public, with "the environment override
  honoured by the wrapper and the example alike", and "an explicit endpoint
  constructor" that the spec did not name.
- **Actual:** `pub fn resolve_endpoint() -> PathBuf` is the single resolver
  (override when set and non-empty, otherwise the default) and both the wrapper
  and the example call it, and `pub async fn Client::with_endpoint(endpoint) ->
  Result<Self, ClientError>` is the endpoint constructor.
- **Impact:** additive, no caller breaks. The daemon effort should call
  `resolve_endpoint` rather than restating the rule, but
  `const ENDPOINT_ENV: &str = "METIS_ENDPOINT"` is private, so it must restate
  the variable name; exporting the constant would remove the drift risk.

**`Client` has no public transport injection point.**
- **Planned:** "`pub struct Client`, holding the generated `DaemonServiceClient`
  over an injected transport. Its API names no transport and no generated stub
  type."
- **Actual:** the field is private, the transport is built inside the
  constructors (`local()` and `with_endpoint()`, both `#[cfg(unix)]`), and the
  public API names neither the transport nor a generated type, as planned. There
  is no way to hand `Client` a transport the crate did not construct.
- **Impact:** on dependents, medium. The later transport tickets extend `client`
  with their own constructor (or expose an injection point) instead of injecting
  from outside, which keeps the spec's "a later transport changes the
  construction, not the caller's code" true but makes every new transport a
  `client` change. On non-Unix, `Client::local` does not exist yet, so a Windows
  build has no local constructor until `windows-named-pipe` lands.

**`ClientError` matches the plan.**
- **Planned:** a typed error over transport and protocol failures, with the
  credential variants arriving with `local-identity-and-acl`.
- **Actual:** `#[non_exhaustive] pub enum ClientError { Call(ConnectError) }`
  with `Display`, `Error::source`, and `From<ConnectError>`. No divergence.

**The endpoint literals are fixed here, and the default is security-relevant.**
- **Planned:** the ticket fixes the two literals; the daemon effort binds the
  same default.
- **Actual:** `default_endpoint()` is `PathBuf::from("/tmp/metis-daemon.sock")`,
  Unix-only and with no runtime-directory or per-user fallback; the override is
  `METIS_ENDPOINT`, and an empty value is treated as unset.
- **Impact:** on dependents, high for one ticket.
  `local-identity-and-acl` requires the socket **and its directory** to be
  created with a configured owner and group and mode 700 or 770. `/tmp` is
  world-writable and sticky (mode 1777), so that directory requirement cannot be
  met at this path, and `serve_uds` unlinks whatever is at the path before
  binding, so a shared directory lets another local user pre-create the path
  between daemon starts. See "User attention needed" below.

### Abstraction usage

- Used/was specified: **yes.**
- The transport ports the ConnectRPC spike's measured shapes rather than
  inventing them: the accept-loop body is `ConnectRpcService` plus a
  `hyper_util` auto (HTTP/1.1 or HTTP/2) builder, the connector is
  `Http2Connection::lazy_with_connector` over a `tower::service_fn` that opens a
  `UnixStream`, the `:authority` is the spike's `local_authority()`, and the
  example keeps the spike binary's "wait for the socket, then call" driver
  shape. The spike's own correction is respected: there is no
  `Server::serve_connection` in connectrpc 0.9.1, and the push path is used
  rather than a `Server` bind.
- The generated surface is used, not restated: `DaemonService`,
  `DaemonServiceExt::register`, `DaemonServiceClient`, and the `metis.v1`
  message types, with `#[allow(refining_impl_trait)]` on the implementation
  block exactly as the arch spec requires. Nothing under `generated/` is
  touched, and no second echo request or response type exists.
- One serve loop and one connector shape: `serve_uds` is a listener front-end
  over `serve_stream`, `uds_transport` is a connector front-end, and no wrapper
  around `connectrpc::Server` was added.
- One deliberate difference from the spike (recorded above):
  `ConnectRpcService::from_arc` rather than `::new`, forced by the non-`Clone`
  router.

### Out-of-scope changes

- Root `Cargo.toml`: the pinned `connectrpc` moved into
  `[workspace.dependencies]` with the same exact pin and features, and
  `api-contracts` now says `connectrpc.workspace = true`. Effective dependency
  graph unchanged; the pin is no longer duplicated once a second crate names it.
- Root `Cargo.toml`: `tokio`, `tower`, `hyper`, and `hyper-util` added as
  workspace entries. `tokio`'s features are narrowed to `io-util`, `macros`,
  `net`, `rt`, `time` (the features the listener, the connector, the skeleton
  wait, and the tests actually name); `rt-multi-thread` is deliberately absent,
  so the example pins `#[tokio::main(flavor = "current_thread")]`.
- `crates/client/Cargo.toml`: `tokio` as a normal dependency (see the skeleton
  finding), plus `http`, `hyper-util`, and `tower` as dev-dependencies for the
  loop-seam test, which drives a duplex pair through the generated client.
- `received_at`: the contract's RFC 3339 string is produced by a hand-written
  days-to-civil routine (`format_rfc3339`, `civil_from_days`, about 40 lines)
  instead of a date crate, so no dependency was added. The spike stamped epoch
  millis; the skeleton follows the contract instead.
- Additive helpers the spec did not name: `skeleton::router()`,
  `skeleton::wait_for_socket()` (bounded poll), and a `Drop`-guarded temporary
  socket directory in the test.
- `docs/dev-env.md`'s "Reproduction" section is filled in with
  `cargo run -p client --example echo`; the arch spec listed `docs/testing.md`
  only, which was also updated.
- The example's non-Unix `main` prints
  `[result] FAIL: the local transport is Unix-only in this build` and exits 1,
  so the example is honest on a platform whose front-end has not landed.
- Additions only. No removals, no generated file edited, no spike or research
  document touched, no second service implementation.

### Ticket doc update needed?

**Yes.** Append to `## Implementation notes` (do not alter the acceptance
criteria):

1. `serve_stream` takes `impl Into<Arc<Router>>` and builds the service with
   `ConnectRpcService::from_arc`, because `connectrpc` 0.9.1's `Router` is not
   `Clone`; an accept loop shares one router through an `Arc`. Later transport
   tickets follow this form.
2. The two literals this ticket fixed: `default_endpoint()` is
   `/tmp/metis-daemon.sock` on Unix, and the override variable is
   `METIS_ENDPOINT`, resolved by `resolve_endpoint()` with an empty value meaning
   unset. The variable name is a private constant, so the daemon effort restates
   the literal.
3. The skeleton service is a public module (`client::skeleton`), not `cfg(test)`
   code, which is what lets the integration test and the example share one
   implementation; it also makes `tokio` a normal dependency of `client`, since
   `wait_for_socket` uses `tokio::time`.
4. `Client` has no public transport injection point: the transport is built
   inside `local()` and `with_endpoint()`, both `cfg(unix)`. A later transport
   extends `client` with its own constructor.
5. `resolve_endpoint()` and `Client::with_endpoint()` are additive public
   functions beyond the arch spec's export list; `with_endpoint` is async and
   returns `Result` so it can become an eager connect without an API break.
6. Manifest changes: `connectrpc` moved to the root `[workspace.dependencies]`
   (same pin, same features), `tokio`/`tower`/`hyper`/`hyper-util` were added
   there, and `client` gained `http`/`hyper-util`/`tower` as dev-dependencies.
7. The skeleton formats `received_at` as RFC 3339 UTC with a hand-written
   days-to-civil routine and no date dependency; the value's shape is asserted
   at the duplex seam.
8. Residual risks: the endpoint literal sits in a world-writable directory, and
   the suite's environment override mutates the process environment (unsafe in
   edition 2024), which is safe today only because one test in that binary reads
   or writes the variable. Both are in the deviation report.

### User attention needed?

**Yes**, two items, and neither blocks this ticket's acceptance criteria:

1. **The endpoint default conflicts with the ACL ticket.** `/tmp` is
   world-writable and sticky, so `local-identity-and-acl` cannot satisfy its
   "socket and its directory with a configured owner and group and mode 700 or
   770" criterion while the socket path is `/tmp/metis-daemon.sock`, and
   `serve_uds` unlinks whatever is at the path before binding, so another local
   user can pre-create the path between daemon starts. Choose one before that
   ticket runs: a daemon-owned directory (for example
   `$XDG_RUNTIME_DIR/metis/daemon.sock`, with a documented fallback), or keep
   `/tmp` and move the directory half of the ownership requirement into that
   ticket. The arch spec says the daemon effort binds the same default, so this
   decision propagates.
2. **The skeleton ships in `client`'s public API.** It is documented as test
   support but is an unconditional `pub mod skeleton`, and it is the only
   reason `tokio` is a normal dependency of `client`. Either accept it until the
   daemon effort removes it, or gate it behind a feature the example and the test
   require. The spec's "test support rather than product API" intent is not
   enforced by the compiler today.

### Verification evidence

- Ran: `devenv shell -- cargo test -p client` -> 4 passed, 0 failed
  (`default_endpoint_is_the_platform_default`,
  `serve_stream_serves_a_router_over_a_duplex_stream`,
  `environment_override_redirects_the_local_constructor`,
  `wrapper_echoes_over_a_temporary_socket`).
- Ran: `devenv shell -- cargo tree -p client -e normal --depth 1` -> direct
  dependencies are `api-contracts`, `connectrpc` 0.9.1, `tokio` 1.53.2; a
  full-tree grep for `tauri`, `ratatui`, `crossterm`, and `egui` matches nothing,
  so the UI-independence criterion holds.
- Read: the full diff, plus `crates/api-contracts/src/transport.rs`,
  `crates/api-contracts/src/lib.rs`, `crates/client/src/lib.rs`,
  `crates/client/src/skeleton.rs`, `crates/client/tests/echo.rs`,
  `crates/client/examples/echo.rs`, and the four manifest and doc hunks.
- Not re-run here: `cargo fmt --check`, `cargo clippy`, and the full workspace
  suite, which the slice verifier owns; the worker's report records them green.
- The working tree carries only the orchestrator's `docs/tasks/state.yaml`
  modification, and nothing is staged.
