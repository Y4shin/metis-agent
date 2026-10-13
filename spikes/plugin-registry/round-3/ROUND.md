# Round 3: simplifying the plugin ABI/FFI and the Rust trait contract

Throwaway prototype round. Not product code, not a committed ABI, no CI.

## The north star for this round (operator's words, condensed)

Get the prototype as close as possible to: **the plugin just gives the host async
tasks that the host can launch and await on its own runtime.** Refine and
SIMPLIFY the C ABI/FFI tables and the Rust trait side: fewer types, fewer table
entries, fewer host-side moving parts, still implementable without Rust. Prefer
deleting surface over adding it. Do not start any ticket or implement-task work.
The operator does a final approval round on the resulting contract shape, so
every lane must say exactly what it would delete and what it would keep.

## Lane table

| Lane | Copy | Question | Measurable outcome |
| --- | --- | --- | --- |
| shape | `/tmp/metis-plugin-registry-round-3/shape` | If plugin futures are the only way the host calls a plugin, how small can the ABI plus the Rust trait surface get, and what exactly disappears? | Full suite green with the legacy callback path compiled out, plus a written v2 contract sketch and a deleted-surface table |
| reclaim | `/tmp/metis-plugin-registry-round-3/reclaim` | How does the host reclaim an abandoned blocking poll, and what is the unload rule? | A reclaim proven once-only on a late poll return, and a demonstrated unload rule with the process still alive |
| ownership | `/tmp/metis-plugin-registry-round-3/ownership` | Is the written ownership rule enough for a non-Rust implementer, and can such an implementer drive the seam end to end? | A real C `.so` compiled with gcc in the devenv shell and driven end to end by the host, plus the ownership clause |
| async-ffi | `/tmp/metis-plugin-registry-round-3/asyncffi` | Do we need the `async-ffi` crate at all, and what does it cost? | The `Drop`-panic exit status, a two-implementation comparison, and a keep/drop recommendation |

## Shared context

- Repo (read-only for you, except your own copy):
  `/home/patric/Projects/metis-agent`.
- Read, in this order, before you write anything:
  `docs/plugin-registry-handoff.md`, `spikes/plugin-registry/README.md`,
  `spikes/plugin-registry/FINDINGS3.md`, `spikes/plugin-registry/SEAM-SPEC.md`,
  and skim `spikes/plugin-registry/FINDINGS2.md`.
- Your copy is the one in the lane table above, and its landed
  `plugin-contract` path is already patched to an absolute repo path so it
  builds. Write ONLY inside your copy.
- Never write to the repository, to another lane's copy, or to any other
  `/tmp/metis-plugin-registry-*` path. Delete nothing anywhere.
- This is a throwaway prototype. The deliverable is a written finding backed by
  checks you actually ran, not an architecture document.

## Harness rules (non-negotiable, from `SEAM-SPEC.md` hygiene)

- `cargo` is only on the devenv shell path:

  ```sh
  cd /home/patric/Projects/metis-agent && devenv shell -- cargo build --release --manifest-path <YOUR COPY>/Cargo.toml
  devenv shell -- cargo run -q --release --manifest-path <YOUR COPY>/Cargo.toml -p proto-host
  ```

  Build the whole workspace first: `cargo run` of the host does not rebuild the
  cdylib member, and the harness refuses to run against a stale library.
  `gcc` and `clang` are also only inside `devenv shell`.
- Baseline to preserve: **92 checks, 0 failures, exit 0**, in debug and
  release. Your additions must keep it green.
- One printed line per check, `PASS`/`FAIL`, stdout flushed per line, non-zero
  exit on any failure.
- Every wait has a short deadline that prints `FAIL` instead of blocking. No
  bare join that can hang.
- A run killed at a timeout must still leave its last marker on disk.
- Report only what you drove. Anything not driven or not measured goes in an
  explicit `Unproven` list, never described as verified.

## Deliverable per lane

Write `FINDINGS-<lane>.md` in your copy **incrementally**, not at the end.

Structure:

1. `Status`: COMPLETE or PARTIAL, with a read-this-first caveat, and the check
   counts in debug and release plus the exit codes.
2. `Answers at the top`: one line per question you were asked.
3. `What was actually driven`: exact commands, check counts, numbers.
4. `Unproven`: everything not driven or not measured.
5. `Recommendation`: one paragraph, and the surface you would delete or keep
   under the north star.
6. `Contract deltas`: for every ABI type, table entry, or trait item you would
   add, change, or remove, name it exactly.

Return from your run: check counts in debug and release, the exact commands,
the numbers, your recommendation, residual leaks or hazards you did not fix,
and the paths of every file you wrote inside your copy.

If you hit a decision that changes the contract shape and the written spec does
not settle it, record it under `Contract deltas` as an open operator decision
and continue. Use `contact_supervisor` only if you are genuinely blocked.

Budget: these lanes have taken one to three hours each. Aim to land your first
measurable outcome early and keep the findings file current as you go; a
partial result with honest unproven lists beats a killed run with nothing on
disk.
