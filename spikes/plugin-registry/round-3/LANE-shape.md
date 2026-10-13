# Lane shape

YOUR COPY: `/tmp/metis-plugin-registry-round-3/shape`

## Question (one)

If plugin futures are the only way the host calls a plugin, how small can the ABI
plus the Rust trait surface get, and what exactly disappears?

This lane is the one the operator's approval round reads first. Make the sketch
exact and copy-pasteable.

## Do this

1. **Decommission measurement.** In your copy, put the legacy `invoke`/`complete`
   callback path behind a cargo feature (for example `legacy-callbacks`, default
   on) and measure the configuration with it OFF: the host suite still passes,
   and you report exactly what becomes dead surface: table entries, ABI types,
   host modules, and the executor seam plus its bounded pool and thread-per-call
   fallback. Delete the dead code in the OFF configuration and report the
   reduction as a table of item, what it was, gone or kept, and why. The
   measurable outcome is the suite green with the legacy path compiled out, plus
   that table.
2. **Trait simplification.** Read
   `/home/patric/Projects/metis-agent/crates/plugin-contract/src/authoring/traits.rs`
   for the landed four-arm `Plugin` trait: `ANSWER`/`DEFERRED`/`CANCELLABLE`/`FAULT`
   consts plus four associated `Shape` types, each with its own request type.
   That four-arm vocabulary is the known defect that caps a plugin at four
   capabilities, and it is the item blocking the SDK from pinning author-facing
   API. Prototype its replacement INSIDE your copy: a single
   capability-name-routed async surface, one request and response shape resolved
   by name, no per-arm consts and no per-arm associated types. Write the exact
   signatures. If the four arms survive for a real reason, name that reason
   against dyn compatibility and object safety, and prove it with a compile
   probe rather than a claim.
3. **Host launch and await.** Give the host ONE launch and await API over the
   seam, shaped like:
   `let task = host.launch(capability_name, request_bytes)?; let response = task.await?;`
   with N tasks in flight on the host's own runtime, no plugin thread and no
   plugin runtime. Drive at least 8 concurrent plugin futures to completion,
   prove the plugin thread and runtime delta is zero, and report the per-call
   cost of the launch-and-await path against the numbers already in
   `FINDINGS3.md` (host-polled ready-on-first-poll 864 ns, reuse 440 ns, full
   path p50 about 5 us, synchronous native call about 660 ns).
4. **Write the v2 sketch** into `FINDINGS-shape.md` as a fenced block: the whole
   plugin table, the whole host table, and the whole authoring trait, as types
   and signatures, with a one-line note per entry saying kept, changed, or new,
   and what it replaced. Keep it implementable without Rust: every field is a
   pointer, an integer, or a function pointer.
5. Report the honest cost of your simplification: what the host must still do by
   hand (poll, waker, cancel, buffer release, teardown ordering) and which of
   those could move into the SDK.

## Measurable outcome

The full suite passes in both configurations, legacy path on and legacy path
feature-gated off, with check counts for each, plus the deleted-surface table,
plus the v2 sketch, plus the 8-concurrent-futures number and the zero-plugin-thread
proof.
