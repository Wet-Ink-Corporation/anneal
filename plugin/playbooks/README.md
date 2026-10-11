# Playbooks

Multi-skill sequences the skills refer to, the templates they write from, and the schemas of the files they keep.

- `schemas/design-tree.schema.json`: the design tree the `grill` skill will keep (it lands with #20); `anneal grill frontier` and `anneal grill confirm` read it (see `docs/grill.md` in the repository).
- `spec.template.md`: the spec template. A spec written from it passes `anneal lint spec <file>`; the rules, the EARS grammar and the constitution format are in the repository's `docs/spec.md`.
