# Playbooks: `anneal playbook show|list`

```text
anneal playbook show <name>
anneal playbook list
```

The templates the skills write from, the schemas of the files they keep and the tables Clef
reads are built into the binary. The plugin holds no data: a skill runs `anneal playbook show
<name>` instead of reading a file under `${CLAUDE_PLUGIN_ROOT}`. The files live in
`crates/anneal-cli/playbooks/` and are included in one place, so the command, `decide`, the
route hook and the lints read the same bytes.

| Name | Media type | What it is |
|---|---|---|
| `spec-template` | `text/markdown` | The spec template `anneal lint spec` checks against (`docs/spec.md`) |
| `plan-template` | `text/markdown` | The plan template `anneal lint plan` checks against (`docs/plan.md`) |
| `design-tree-schema` | `application/json` | The JSON Schema of the `grill` skill's design tree (`docs/grill.md`) |
| `plan-schema` | `application/json` | The JSON Schema of `lint plan`'s `data.plan` (`docs/plan.md`) |
| `route` | `application/json` | The phases and sizes Clef's D3 chooses from (`docs/hooks.md`) |
| `clef-thresholds` | `application/json` | Each decision point's model, fallback and bands (`docs/decide.md`) |

## Output

`show` writes the usual envelope (`command` is `playbook show`) with:

```json
{"name": "spec-template", "media_type": "text/markdown", "content": "# Spec: <feature>\n..."}
```

`content` is the file's text exactly as built in. To write a spec, a plan or a thresholds copy
from it, save `data.content` (for example `anneal playbook show spec-template | jq -r
.data.content > spec.md`).

`list` writes `data.playbooks`, one `{name, media_type, summary}` per playbook in the order of
the table above.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | ok |
| 2 | usage error: an unknown name or subcommand; nothing on stdout, one `ERROR` line on stderr |

A green result proves the text is the one built into this binary. It does not prove that the
playbook fits the task, or that a file written from it passes its lint: run `anneal lint spec`
or `anneal lint plan` on the file.

## Thresholds

`anneal decide` and `anneal calibrate` use the built-in `clef-thresholds` unless
`--thresholds <file>` names a copy. Hooks and `grill classify` have no `--thresholds` and
always use the built-in table; no command reads a thresholds file from the plugin root.
