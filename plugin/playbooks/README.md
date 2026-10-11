# Playbooks

Multi-skill sequences the skills refer to, the templates they write from, and the schemas of the files they keep.

- `schemas/design-tree.schema.json`: the design tree the `grill` skill will keep (it lands with #20); `anneal grill frontier` and `anneal grill confirm` read it (see `docs/grill.md` in the repository).
- `spec.template.md`: the spec template. A spec written from it passes `anneal lint spec <file>`; the rules, the EARS grammar and the constitution format are in the repository's `docs/spec.md`.
- `plan.template.md`: the plan template. A plan written from it passes `anneal lint plan <file>`; `anneal plan approve <file>` records the human's approval by hash and `anneal plan check <file>` tells whether it is still current. The format, the rules and the exit codes are in the repository's `docs/plan.md`.
- `schemas/plan.schema.json`: the structure `anneal lint plan` reads from a plan (its `data.plan`): the spec link and each task's id, title, files, test command and blockers.
