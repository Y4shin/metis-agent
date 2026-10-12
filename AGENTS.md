# Agent conventions

This repository is onboarded onto the task-workflow. These conventions apply
to any agent working here.

## Where work lives

- Work items are files under `docs/tasks/`. There is no external issue
  tracker.
- The task graph starts at `docs/tasks/maps/<slug>/map.md` (created by
  `/skill:wayfinder`).
- Bug reports live under `docs/bugs/`, with resolved ones archived under
  `docs/bugs/archive/`.
- Rejected requests are recorded under `docs/tasks/out-of-scope/`, one file
  per rejected request.
- Decisions are recorded as ADRs under `docs/adr/`.
- The project glossary is `CONTEXT.md` at the repo root. Domain terms go
  there, not in prose scattered across documents.

## Getting oriented

- `/skill:task-workflow-overview` is the router: it maps every skill and how
  the flows relate. Start there.

## Skills: buckets, promotion, and invocation

These rules apply to Pi skill packages. This repo does not currently ship a
`skills/` tree, so they are dormant until one is added.

- Skills live in bucket folders: `engineering/` and `productivity/` are the
  promoted buckets, plus `misc/`, `in-progress/`, and `deprecated/`.
- A promoted skill must appear in the top-level `README.md` and in the
  package manifest. Non-promoted skills must not appear in either.
- Every skill is either user-invoked (`disable-model-invocation: true`,
  reachable only when the human types the name) or model-invoked (omit the
  flag). A skill is one or the other, never both. A user-invoked skill may
  invoke model-invoked skills, but never another user-invoked one.

## Prose style

No em-dashes anywhere in this repo's prose (documents, `README.md`,
`CHANGELOG.md`, ADRs, code comments). Where a sentence reaches for one,
rewrite it with a comma, colon, period, parentheses, or a conjunction,
whichever the sentence actually wants. Never do a blind character
substitution.

## Skill and tool invocation

- In Pi, a model-invoked skill is reached by the model invoking it or by
  `/skill:<name>`. A user-invoked skill is reached only by the human typing
  `/skill:<name>`.
- When a step needs another skill, name it explicitly, one skill per
  instruction.
