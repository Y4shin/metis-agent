---
type: task
subtype: prototype
title: Does spec-first generated server and client code work well enough?
status: stable
workflow_state: done
blocked_by: []
---

## Question

Does generating both sides from `contracts/openapi.yaml` with `openapi-generator`
produce something this repository can actually use? Concretely:

1. Does the `rust-axum` server generator emit a crate that compiles inside this
   devenv, and can the generated router be served over a Unix domain socket
   through hyper rather than over TCP?
2. How is a `text/event-stream` response represented by the generator, and how
   much of the streaming, the `Last-Event-ID` resume, and the heartbeat stays
   handwritten?
3. Does the generated Rust client accept a custom transport (reqwest's
   `unix_socket` and `windows_named_pipe`), or does it need a wrapper to inject
   one?
4. The structural risk: two generated crates mean two sets of DTOs. How do they
   interact, and how much conversion code does the boundary between the
   generated server types and the generated client types require?
5. What is the real toolchain cost: which JVM, how the generator is pinned, how
   long generation takes, and does the generated code pass `cargo fmt --check`
   and `cargo clippy --all-targets -- -D warnings` under the workspace lints, or
   does it need module-level allows?
6. What is the exact drift-check command that regenerates and fails on a diff?

## Alternatives worth comparing

- `openapi-generator` for both sides, which is the path being tested.
- Generate the client only, with a handwritten axum server and a conformance
  test that the server matches the document.
- Generate models only with `typify`, and handwrite both HTTP layers.

Measure the handwritten glue in each case, in lines, so the comparison is not a
matter of taste.

## Smallest artifact that answers it

A pinned generator invocation and a small spec owned by the spike, kept under
`spikes/openapi-codegen/` so the real `contracts/` placeholders are untouched.
The spec needs one ordinary request and one SSE endpoint. The spike also holds
the generated crates, a regeneration script or xtask, and a tiny binary that
serves the generated router over a Unix socket and opens the SSE stream through
the generated client. The spike directory must not be added to the root
workspace members list.

## Environment note

Java is not in `devenv.nix` today. Obtain a JVM for the spike, for example with
`nix shell nixpkgs#jdk21`, or by adding `pkgs.jdk` to `devenv.nix`. Report which
route worked, and leave any `devenv.nix` change in place only if the outcome is
conclusive; the parent decides whether it keeps it.

## Who reacts

The user reviews the numbers and the generated code. The answer settles
spec-first versus code-first, which reopens or confirms the
`contract-generators` recommendation.

## What it unblocks

The `contract-generators` decision, the `api-contracts` crate layout, the CI
drift-check step, and the spec's contract architecture. An inconclusive result
is acceptable: record the measured friction, abandon the spike, and keep the
decision open.

## Completion evidence

`findings.md` beside this task, with frontmatter `type: findings`, `status:
stable`, recording each of the six questions with its answer, the measured glue
per alternative, the exact commands including how the JVM was obtained, the
pinned generator version, and the generated code's fmt and clippy results. The
spike source stays in `spikes/openapi-codegen/` as reference.
