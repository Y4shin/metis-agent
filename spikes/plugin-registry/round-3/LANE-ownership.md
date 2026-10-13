# Lane ownership

YOUR COPY: `/tmp/metis-plugin-registry-round-3/ownership`

## Question (one)

Is the written ownership rule for the two service entries enough for an
implementer that is not Rust, and can such an implementer drive the seam end to
end?

## Context

Open items O3 and O10 of the handoff. The prototype's reading is that the
`service_request` payload is plugin-owned and released by the host, and the
`service_complete` payload is host-owned and borrowed for the call. That is an
interpretation, not contract text, and no C plugin has ever been compiled for
either async path. Only a hand-written Rust stand-in exists.

## Do this

1. **Ownership clause.** Write the rule as contract text: for the ready path
   `out_value` buffer, for `service_request`, and for `service_complete`, name
   who allocates, who frees, with which call, and when the pointer stops being
   valid. State what happens when the plugin's future is dropped while a service
   reply is in flight.
2. **A real C implementer.** Build a C shared library, compiled with `gcc` or
   `clang` inside `devenv shell` (both are present: gcc 16.2.0, clang 21.1.8),
   under the prototype's own flags `-std=c11 -Wall -Wextra -Werror`. It must
   implement the plugin entry point and the plugin table by hand in C, using the
   cbindgen header at `crates/proto-abi/include/`, including `invoke_future`, the
   `poll` function, the `drop` function, and the waker vtable, plus one
   capability that answers end to end and one `service_request` timer round trip
   answered through `service_complete`. No Rust glue, no Rust async machinery, no
   Rust helper. Load the C `.so` with the host and drive it on the real path.
3. **Accounting.** Prove from the C side that each side frees only what it
   allocated: counters in the C code for allocations and frees, checked against
   the host side counters, with a zero-leak and zero-double-free result at
   teardown.
4. **Friction list.** Report every place the C implementer had to know something
   the ownership clause or the ABI header did not state: a size rule, a lifetime,
   a reentrancy rule, a null rule, an alignment rule. Each one is a contract
   delta for the approval round. Also report how many lines of C the minimal
   implementer needed, because that is the real measure of whether the ABI is
   simple.

## Measurable outcome

A C `.so` compiled and driven end to end by the host, with the ownership clause
in writing and the accounting counters at zero. If the ABI as it stands cannot be
implemented from C, that finding IS the deliverable: name the exact type or rule
that blocks it.
