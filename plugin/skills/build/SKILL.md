---
name: build
description: Use when an approved task is next, even if it looks small; build it test-first, red then green, and mark it done only by its test command. Do NOT use for splitting a spec into tasks (use `plan`), writing the spec (use `spec`) or a failure with an unknown cause (use `diagnose`).
---

# Build

## Overview

Builds one approved task test-first with a single approval up front and hard stops after it: the hooks check every edit batch, the task's test proves it done, and CI runs the full suite. Check placement and the `CONSTRAINTS.md` format are in references/constraints.md; the loop is in references/tdd.md.

- You MUST enter build mode with `anneal mode build --plan <plan> --tree <tree>` on a clean tree before editing source; the `preconditions` hook denies source edits until the plan is approved and current.
- You MUST write the failing test first and see it fail; the `preconditions` hook allows test files before source.
- You MUST mark a task done only with `anneal task done <id> -- <test command>`; it records `task_done` only when the command exits 0.
- You MUST NOT lower a floor in `.anneal/ratchets.json` or remove a test; the `fast-check` hook blocks the batch and `anneal floor-guard` fails the diff.

## When to use

- A plan passes `anneal plan check` (exit 0) and one of its tasks has every blocker done.
- Not when the task list is missing (use `plan`), the spec is missing (use `spec`), or something broke and nobody knows why (use `diagnose`).

## Process

1. **Check the plan.** Run `anneal plan check <plan>`; exit 4 means it changed, so it goes back to the human. It's working if it exits 0.
2. **Enter build mode.** Commit or stash first, then run `anneal mode build --plan <plan> --tree <tree>`. It's working if `data.clean` is true.
3. **Read the task.** Its `Files:`, its `Test:` command and the spec's criteria it serves. It's working if you can say which test proves it.
4. **Red.** Write the test, run the task's command, watch it fail for the reason the criterion names. It's working if the failure message names the missing behaviour.
5. **Green.** Write the least code that passes. Read every `fast-check` note and block after each batch of edits. It's working if the command passes and no block is open.
6. **Refactor.** Clean names and duplication with the tests green. It's working if the command still passes.
7. **Gate.** In a Rust repository (a `Cargo.toml` at the root), run `temper:gate` at each task boundary. It's working if the gate is green.
8. **Done.** Run `anneal task done <id> -- <test command>`, then check the work with `verify` before you report it. It's working if it exits 0 and the ledger shows `task_done`.

## Rationalizations

| Excuse | Reality |
|---|---|
| "It's too small to test first" | Then the test is small too. Write it. |
| "I'll write the tests after" | A test written after passes by construction. |
| "The ratchet is wrong, I'll lower it" | Lowering a floor is the human's call, not the build's. |
| "The fast-check note is noise" | Read it; it is the cheapest place to catch the problem. |
| "The task is done, the test is flaky" | Done is a green `anneal task done`, nothing else. |

## Red flags

- Source edited before a failing test exists.
- A ratchet value, a coverage floor or a timeout changed in the same diff as the code.
- A test deleted, skipped or loosened to get green.
- Marking a task done by hand, or by editing the ledger.

## Verification

- `anneal plan check <plan>` exits 0.
- `anneal floor-guard` exits 0.
- `anneal ledger query --type task_done` shows the task, after a green `test_run`.
