---
type: task
subtype: research
title: What the plugin control plane's counters were for, and whether anything consumes them
status: deprecated
workflow_state: done
blocked_by: []
---

## Question

`docs/plugin-contract.md` names "counters" once, in section 14 item 7, as one
member of the `metis.*` control plane that moves onto the futures path, and once
indirectly in section 11 rule 6, where a quarantined plugin's "counters stay
honest". Nothing in the tree defines the term. Answer three things:

1. Where did the word come from, and what did it refer to at each place it
   appears, including the archived architecture spec, the spikes, and any plan
   or harness text?
2. Does anything consume it today? The candidates are the plugin-side
   accounting in `crates/plugin-contract/src/authoring/alloc.rs`
   (`Accounting { allocs, frees, live, violations }`), the host's waker counts,
   the ordering ledger, the fixture's ledger, and the ABI harness's counter
   checks.
3. Is it orphaned? If nothing consumes it and nothing needs it, recommend
   removal. If something needs it, say exactly what shape that need requires.

## Decision this unblocks

The counters question in `configuration-and-host-context`, which cannot be
settled while the term is undefined. That decision is whether counters get a
dedicated always-present table entry, stay a reserved-namespace capability, or
leave v1. The operator's reading is that counters may have been intended for
atomic request-id creation to map running futures, which would make them host
state rather than plugin state, and that possibility must be checked rather
than assumed.

## Trusted source boundaries

- Local, primary: `docs/plugin-contract.md` (sections 11 and 14),
  `docs/tasks/archive/metis-foundations/arch-spec.md`,
  `docs/tasks/archive/metis-foundations/tickets/plugin-contract-v1/` and its
  deviation report, `docs/adr/0002-plugin-contract-futures-only.md`,
  `spikes/plugin-registry/FINDINGS*.md`, `spikes/plugin-registry/round-3/`,
  `docs/rough-plan.md`, `crates/plugin-contract/src/authoring/alloc.rs`,
  `crates/plugin-fixture/src/`, `crates/plugin-fixture/src/bin/abi-harness.rs`,
  and `crates/plugin-runtime/src/`.
- History counts as evidence here: `git log` and the archived ticket docs are
  how a deleted or renamed concept is traced.
- No external source is expected to help. This is a question about this
  repository's own intent.

## Evidence required for completion

- Every occurrence of the term, with the file and line, and what it referred to
  in each place.
- Every consumer, or an explicit statement that there is none, with the search
  that establishes it.
- A recommendation: remove the term and the feature from the contract text, or
  keep it with a named shape.
- If it is kept, what the shape must be, and which of the two readings holds:
  plugin-owned accounting reported to the host, or host-owned request-id and
  future bookkeeping.
- Every material claim cited to a repository path, with the git revision where
  the claim is about history.

## Dependent tasks

`configuration-and-host-context` waits on this answer for its counters
sub-decision. If the answer is "remove it", the contract text in
`docs/plugin-contract.md` section 14 item 7 needs the amendment that task
already owes, and the effort's ticket list loses the counters entry.
