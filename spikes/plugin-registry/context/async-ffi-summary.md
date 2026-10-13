Implement async Rust shared-library plugins using the existing C ABI and Rust trait wrapper.

**Recommendation:** use [`async-ffi`](https://docs.rs/async-ffi/latest/async_ffi/) to return `FfiFuture<AbiSafeResult>` from plugin calls. The host awaits or spawns these futures on its executor. Expose host-owned async services—HTTP, timers, storage, subprocesses, and task spawning—through ABI-safe callbacks so plugins can await them without owning a runtime.

**Important:** an FFI-safe future does not automatically share Tokio runtime context. Independently linked copies of Tokio can have separate thread-local state, so plugin-side `tokio::spawn`, timers, and Tokio-dependent libraries may fail even when the host polls the future. Passing a Tokio `Handle` across the ABI is not a stable, supported fix. [Upstream discussion](https://github.com/tokio-rs/tokio/issues/6927)

Alternatives:

- **`abi_stable` + `async-ffi`:** stable types, trait interfaces, and load-time compatibility checks; its loading model excludes unloading.
- **`stabby`:** an integrated stable-ABI framework with trait objects and future support.
- **`safer-ffi`:** C-compatible interfaces with future/executor abstractions.
- **Wasmtime components:** portable, sandboxed plugins with host-driven async execution; replaces native shared libraries.
- **Long-lived plugin runtime:** pragmatic fallback if plugins need arbitrary Tokio-dependent crates. Avoid per-request OS threads.
- **One dynamically linked Tokio:** possible for tightly controlled builds, but brittle for independently distributed plugins.

Ensure all boundary types are ABI-safe; define ownership, cancellation, panic handling, and ABI versioning. Keep libraries loaded until every plugin future, callback, and destructor is gone. Validate using independently built plugins, concurrent calls, host-backed I/O/timers, and cancellation.
