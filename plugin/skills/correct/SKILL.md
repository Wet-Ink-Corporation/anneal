---
name: correct
description: Use after a mistake, review finding or correction you never want repeated, even if a line in CLAUDE.md seems enough; fix it at the highest level that holds and give every rule an enforcer. Do NOT use for finding a failure's cause (use `diagnose`) or proving work done (use `verify`).
disable-model-invocation: true
argument-hint: "[the mistake, finding or correction]"
---

# Correct

## Overview

A mistake becomes a check, never a sentence. Fix it at the highest level that holds, and write a rule down only with the hook, lint or test that enforces it.

- You MUST name the enforcer of every rule you write, and run `anneal lint rules <file>` until it exits 0.
- You MUST emit a hook entry, lint configuration or test from the retro, never prose alone; `anneal lint rules` fails a rule whose enforcer is missing or cannot be found.

## When to use

- A review, a gate or the human caught a mistake that could recur.
- The same correction was given twice.
- Not to find why something broke (use `diagnose`), nor to prove work done (use `verify`).

## The ladder

Take the first rung that would have stopped the mistake.

| Rung | The fix | Its enforcer |
|---|---|---|
| 1 Architecture | Make the mistake impossible: remove the path, split the module | the code itself; a test pins it |
| 2 Types | Make it unrepresentable: a newtype, an enum, a typestate | the compiler; a test pins it |
| 3 Lint | A lint whose message names the fix | `anneal <command> ...`, or a hook that runs it |
| 4 Test | A test that fails on the mistake | `test <file>::<fn>` |
| 5 Docs | Prose, last and never alone | rung 3 or 4 beside it |

## Process

1. **Apply the bar.** Ask: would a future engineer, with no memory of today, still repeat this? If not, fix the instance and stop. It's working if you can say why it would recur.
2. **Climb the ladder.** Fix at the first rung that would have caught it. It's working if the fix makes the original mistake fail a check, not a reader.
3. **Emit the enforcer.** Write the test, lint or hook. A new hook gets its `hooks/hooks.json` entry and `hooks.metadata.json` row together; `.anneal/config.json` (`check_fast`) is the human's file, so hand them the exact JSON. It's working if the enforcer fails on the mistake and passes on the fix.
4. **Record the rule.** Add one item under `## Rules` in the rule document (`docs/rules.md` unless the repository keeps another): the rule, then `Enforcer:` and, in backticks, `hook <id>`, `anneal <command> ...` or `test <file>::<fn>`. It's working if a stranger can find the check from the line.
5. **Lint the rules.** Run `anneal lint rules docs/rules.md` (add `--hooks <plugin>/hooks/hooks.metadata.json` when the repository has none). It's working if it exits 0.
6. **Report.** Name the rung, the enforcer and the rule's line. It's working if the human can run the enforcer unaided.

## Rationalizations

| Excuse | Reality |
|---|---|
| "A note in CLAUDE.md will do" | Notes are read once and forgotten; a check runs every time. |
| "A test is enough, the type change is too much work" | A lower rung leaves the mistake writable. Climb as high as it holds. |
| "It was a one-off" | Then the bar says stop, and nothing is written. Say so. |

## Red flags

- A rule with no `Enforcer:`, or one naming a check that does not exist.
- A docs-only fix for a mistake a type or test could stop.
- A hook entry without its metadata row.
- Editing `.anneal/config.json` yourself.

## Verification

- `anneal lint rules docs/rules.md` exits 0.
- The enforcer fails on the original mistake and passes on the fix.
- The rule's line names that enforcer.
