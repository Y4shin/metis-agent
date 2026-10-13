# Lane async-ffi

YOUR COPY: `/tmp/metis-plugin-registry-round-3/asyncffi`

## Question (one)

Do we need the `async-ffi` crate at all, and what does it cost?

## Context

Open item O4 of the handoff. Only one implementation used `async-ffi`, and its
`Drop`-panic path aborts the process, which our shim cannot contain. The operator
wants the smallest surface, so this lane must say whether the crate earns its
place.

## Do this

1. **Drop-panic probe.** Build a probe where a future's `Drop` panics while it is
   driven through the `async-ffi` wrapper. Run it in a child process and record
   the exit status and any stderr. Our own `poll` and `drop` shims are supposed
   to be the boundary and wrap the author's future in `catch_unwind`; if a
   `Drop` panic reaches `std::process::abort`, that is a hard finding for a
   daemon that holds many plugins and must not die because of one. State whether
   the abort is containable at all from outside the crate, and whether the
   drop-panic path is reachable from a plugin that only ever returns `Ok`.
2. **Two-implementation comparison.** Both implementations of the same capability
   already exist in your copy: the `async-ffi` one and the hand-written
   no-`async-ffi` vtable one. Compare them honestly: hand-written unsafe lines and
   total lines for each, the ABI types each needs, the symbol or binary
   footprint, the compile-time cost, and per-call cost in the same harness. Reuse
   the existing benches; do not invent new ones unless one is missing.
3. **Recommend.** One paragraph: keep or drop `async-ffi`. Name exactly what an
   author loses if it is dropped, with the two code shapes side by side and their
   line counts, what a non-Rust implementer gains, and what the daemon gains. If
   the answer is keep, name the containment rule the host must then impose. If
   the answer is drop, name the minimum hand-rolled helper the SDK should ship so
   authors do not each rewrite the vtable.

## Measurable outcome

The drop-panic exit status, the two-implementation comparison numbers, and a
one-paragraph keep or drop recommendation with the code shapes side by side.
