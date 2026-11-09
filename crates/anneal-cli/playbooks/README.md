# Playbooks

The templates the skills write from, the schemas of the files they keep, and the tables Clef's hooks read. Each file is built into the `anneal` binary (`src/playbook.rs` includes it, and every reader takes it from there), so a plugin holds no data and a skill asks the binary:

```
anneal playbook show <name>
anneal playbook list
```

`show` writes the usual JSON envelope with `data.name`, `data.media_type` and `data.content`, the file's text exactly as built in; an unknown name is a usage error (exit 2). See `docs/playbook.md` in the repository.

| Name | File | What it is |
|---|---|---|
| `spec-template` | `spec.template.md` | The spec template. A spec written from it passes `anneal lint spec <file>`; the rules, the EARS grammar and the constitution format are in `docs/spec.md`. |
| `plan-template` | `plan.template.md` | The plan template. A plan written from it passes `anneal lint plan <file>`; `anneal plan approve <file>` records the human's approval by hash and `anneal plan check <file>` tells whether it is still current (`docs/plan.md`). |
| `design-tree-schema` | `schemas/design-tree.schema.json` | The design tree the `grill` skill keeps; `anneal grill frontier`, `anneal grill classify` and `anneal grill confirm` read it (`docs/grill.md`). |
| `plan-schema` | `schemas/plan.schema.json` | The structure `anneal lint plan` reads from a plan (its `data.plan`): the spec link and each task's id, title, files, test command and blockers. |
| `route` | `route.json` | The phases Clef's D3 chooses from, each with the entry skill that starts it, the phase that stands for "no skill", and the four size levels D3 scores with the depth each calls for. `anneal hook route` reads it (`docs/hooks.md`). |
| `clef-thresholds` | `clef-thresholds.json` | Each Clef decision point's model (`clef-flash` or `clef`, mapped to Workers AI ids in `models`), its fallback when Clef gives no answer, and per question type the bands that turn a probability into an action. `anneal decide` and `anneal calibrate` use it unless `--thresholds <file>` names a copy; hooks always use it (`docs/decide.md`). |

Editing a file here changes the binary: the tests that read these files (`tests/it/playbook.rs`, the schema agreement tests, the template lint tests) run against the new text.
