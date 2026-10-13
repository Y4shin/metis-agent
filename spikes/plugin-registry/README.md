# Plugin registry prototype

Throwaway prototype. Not product code, not a committed ABI, and not wired into
CI. It exists to answer design questions about the native plugin boundary, and
it is kept in the repository so the questions can be picked up again with the
findings attached.

## What it demonstrates

- A hand-written C ABI for native plugins: a size-prefixed, append-only plugin
  function table and host callback table, opaque instance handles, plugin-owned
  buffers released only by the plugin, and a frozen status space.
- A host-side plugin registry holding `Box<dyn Plugin>` over two adapters: a
  native in-process plugin and an ABI-loaded cdylib, with routing by capability
  name, per-capability version gating, duplicate-name refusal, enable and
  disable, and unregister.
- Both-direction payload validation at the registry, against JSON Schema the
  plugin publishes with each capability.
- A host-polled future seam: the plugin returns an opaque context plus
  `poll`/`drop` and waker vtables, and the daemon polls the plugin's future on
  its own executor, so the plugin needs no executor and no thread. One plugin
  capability uses `async-ffi`; another implements the same vtable by hand as the
  stand-in for a non-Rust implementer.
- The legacy `Pending`/`complete` callback path kept alongside, with its
  per-call work routed through an executor seam: a bounded pool by default and
  thread-per-call as the fallback.

## How to run it

`cargo` is only on the devenv shell's path.

```sh
cd <repo root>
devenv shell -- cargo build --release --manifest-path spikes/plugin-registry/Cargo.toml
devenv shell -- cargo run -q --release --manifest-path spikes/plugin-registry/Cargo.toml -p proto-host
```

Build the whole workspace first: `cargo run` of the host does not rebuild the
cdylib member, and the harness refuses to run against a stale or missing
library. The host prints one line per check, `BENCH` lines for the measurements,
and exits non-zero on any failure. The same commands work in debug by dropping
`--release`.

The prototype also depends on the real `crates/plugin-contract` by relative
path, so it drives the landed ABI types rather than a copy.

## Findings and open questions

- `FINDINGS.md`: the registry shape, dyn compatibility, type erasure, and the
  first validation numbers.
- `FINDINGS2.md`: the async and lifecycle surface, the plugin registry proper,
  and the descriptor and naming fixes.
- `FINDINGS3.md`: the folded host-polled future seam, the hazard handling, the
  versioning and compatibility evidence, and the measurements.
- `SEAM-SPEC.md`: the written contract the seam implements, which every lane in a
  prototyping round works against.
- `context/async-ffi-summary.md`: the source summary of the async approaches that
  started this work.
- Still open, recorded in `FINDINGS3.md`: reclaiming an abandoned blocking poll;
  unloading while a blocked poll is still inside plugin code; payload ownership
  for the two host-service entries; and the fact that only one implementation
  exercised `async-ffi`.

The surrounding design discussion, the decided points, and the full list of open
questions live in `docs/plugin-registry-handoff.md`.
