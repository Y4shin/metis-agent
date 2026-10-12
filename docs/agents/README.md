# docs/agents/

Per-repo configuration that the engineering skills read. `setup-workflow`
populates this directory during initial setup; edit the files directly to
reconfigure.

The files that live here (adapted to the `docs/tasks/` substrate):

- `issue-tracker.md` (or equivalent): how skills read and write work items.
  This repo uses `docs/tasks/` files, not an external tracker; the file
  records that convention.
- `domain.md`: domain doc layout (single-context vs multi-context) and
  consumer rules for reading `CONTEXT.md` and `docs/adr/`.
- `triage-labels.md`: the label strings mapped to the canonical triage
  roles, when triage is configured.

These are created lazily: `setup-workflow` writes them when it runs, and
they are extended as adoption lands.
