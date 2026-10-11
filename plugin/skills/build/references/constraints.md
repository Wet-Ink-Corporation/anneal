# Constraints and check placement

A repository states its constraints in `CONSTRAINTS.md` at its root, and places every check by
its cost. Cheap checks run often; expensive ones run rarely, but always before merge.

## Check placement by cost

| Tier | Budget | Runs | Where | Examples |
|---|---|---|---|---|
| Fast | under 5 s | after every batch of edits | `PostToolBatch` (`anneal hook fast-check`) | `check:fast`, `anneal floor-guard`, avoided terms, `anneal lint spec`/`lint plan` |
| Task | under 90 s | at a task boundary, before Stop | `anneal task done <id> -- <cmd>`, Stop hooks | the task's `Test:` command, `temper:gate` in Rust |
| Full | no limit | on every push | CI | the whole suite, every platform, supply chain |

A check that does not fit its tier's budget moves down a tier; it is never dropped. The fast
tier runs once per batch, not once per edit: Claude Code fires `PostToolBatch` once after every
call of a batch of parallel tool calls has resolved, so ten parallel edits cost one run.

## `CONSTRAINTS.md`

One section per constraint, each naming the check that enforces it and its tier:

```markdown
## C1: No network in unit tests

- Why: unit tests run offline in CI and on planes.
- Check: `cargo test --lib` with the network namespace off.
- Tier: task
```

| Field | Meaning |
|---|---|
| `## C<n>: <title>` | A stable id and a one-line rule |
| Why | The failure it prevents, in one line |
| Check | The command, hook or CI job that enforces it; a constraint with no check is a wish |
| Tier | `fast`, `task` or `full`, by the table above |

## The fast command and the ratchets

`.anneal/config.json` names the fast command as an argv array, run without a shell, killed after
5 s:

```json
{"check_fast": ["cargo", "check", "--quiet"]}
```

`.anneal/ratchets.json` holds the numeric floors that may only move the good way:

```json
{"version": 1, "ratchets": {
  "warnings": {"value": 12, "direction": "down"},
  "coverage": {"value": 81.5, "direction": "up"}}}
```

`down` means lower is better, `up` higher is better. `anneal floor-guard` compares each ratchet
with the merge base: one moved the wrong way, removed or turned around is a lowered floor, and
the `fast-check` hook blocks on it. Tighten a ratchet in the same change that earns it; loosening
one is the human's decision.
