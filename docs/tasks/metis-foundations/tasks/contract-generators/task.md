---
type: task
subtype: research
title: Which contract generators to use, where generated code lives, and how CI proves it is current
status: stable
workflow_state: done
blocked_by: []
---

## Question

Which generators produce the Rust code in `crates/api-contracts` from
`contracts/openapi.yaml` and `contracts/asyncapi.yaml`, where does the
generated output live, and what exact command in CI proves the checked-in
generated code is current?

Three questions have to be answered together because they constrain each
other:

1. Generator choice per document: is there a usable OpenAPI to Rust generator,
   and is there a usable AsyncAPI to Rust generator, or do AsyncAPI channels
   and messages get handwritten types over generated schemas?
2. Placement: the output path inside `crates/api-contracts`, and the line
   between generated code and handwritten behaviour, so that handwritten code
   never lives in a generated file.
3. Verification: the regeneration command and the CI check that fails when the
   committed output disagrees with `contracts/`.

## Decision this unblocks

It unblocks the `api-contracts` implementation tickets, the `client`
implementation, the walking skeleton, and the CI changes that enforce
regeneration. Until it is answered, nobody can write a Rust type against the
contracts without guessing.

## Trusted source boundaries

- Primary: the official documentation, repositories, release notes, and issue
  trackers of each candidate generator, at a pinned release.
- Local: `contracts/openapi.yaml`, `contracts/asyncapi.yaml`,
  `contracts/schemas/`, `crates/api-contracts/`, `Cargo.toml`, `devenv.nix`,
  `.github/workflows/ci.yml`, and the generated-code conventions in
  `docs/rough-plan.md` ("api-contracts", "Network contracts").
- Reproduction counts as evidence: run each viable candidate inside the devenv
  shell and try to compile its output. Secondary write-ups may point at
  candidates but must be confirmed against the primary source.

Candidate families to check, without prejudging the answer: `openapi-generator`
(Rust and Rust server templates), `progenitor`, `paperclip`, and any
Rust-native schema first alternative for HTTP; `asyncapi-generator` and
`modelina` for AsyncAPI, plus the "generate models only, handwritten channels"
fallback. Record why each rejected candidate fails.

## Evidence required for completion

- A candidate table with: supported specification versions (OpenAPI 3.1,
  AsyncAPI 3), produced Rust artifacts (models, client stub, server stub),
  whether the generated client can be pointed at a custom connector (required
  for Unix socket and named pipe transports, which has to be a hard
  requirement, not a nice-to-have), maintenance status, licence, and toolchain
  cost for CI (a Node or JVM dependency is a real cost in a Rust-only repo).
- A reproduced generation run from a minimal representative contract (one
  request, one event), and a compile of its output.
- A recommendation: one generator per document, or models-only generation plus
  handwritten channel code, with the reason.
- The output path and the generated versus handwritten boundary.
- The exact CI check command and how it fails on staleness.
- Every material claim cited to a URL or a repository path.

The current placeholder contracts are empty, so the run needs a minimal
representative document. If that document does not exist yet, create it in a
scratch directory for the run and say so; do not treat it as delivered work.

## Dependent tasks

The `api-contracts` implementation tickets, the `client` connector work, the
walking skeleton, and the CI workflow update all wait on this answer. If the
answer is "AsyncAPI has no usable Rust generator", that is a decision input for
the realtime design task and has to be recorded as such.
