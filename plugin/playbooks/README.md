# Playbooks

Multi-skill sequences the skills refer to, the templates they write from, and the schemas of the files they keep.

- `schemas/design-tree.schema.json`: the design tree the `grill` skill will keep (it lands with #20); `anneal grill frontier`, `anneal grill classify` and `anneal grill confirm` read it (`classify` writes Clef D7's `is_decision` into it once D7 enforces) (see `docs/grill.md` in the repository).
- `spec.template.md`: the spec template. A spec written from it passes `anneal lint spec <file>`; the rules, the EARS grammar and the constitution format are in the repository's `docs/spec.md`.
- `plan.template.md`: the plan template. A plan written from it passes `anneal lint plan <file>`; `anneal plan approve <file>` records the human's approval by hash and `anneal plan check <file>` tells whether it is still current. The format, the rules and the exit codes are in the repository's `docs/plan.md`.
- `schemas/plan.schema.json`: the structure `anneal lint plan` reads from a plan (its `data.plan`): the spec link and each task's id, title, files, test command and blockers.
- `clef-thresholds.json`: each Clef decision point's model (`clef-flash` or `clef`, mapped to Workers AI ids in `models`), its fallback when Clef gives no answer, and per question type the bands that turn a probability into an action. `anneal decide` reads it; the format and the defaults are in the repository's `docs/decide.md`.
- `route.json`: the phases Clef's D3 chooses from, each with the entry skill that starts it (`null` until that skill lands, or for `answer`, a prompt that needs no workflow), the phase that stands for "no skill", and the four size levels D3 scores with the depth each calls for. It is built into the `anneal` binary, which `anneal hook route` reads it from; the hook is documented in the repository's `docs/hooks.md`.
