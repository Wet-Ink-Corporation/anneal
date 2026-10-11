---
name: plan
description: Use when a spec is ready, even if the work seems obvious; split it into tasks, each with its files, one test command and its blockers, then get the plan approved. Do NOT use for writing the spec (use `spec`), a new or vague request (use `grill`) or building an approved task (use `build`).
---

# Plan

## Overview

Turns a spec with no open questions into tasks a fresh session can build test-first, and gets the human's approval recorded by hash. The template is `${CLAUDE_PLUGIN_ROOT}/playbooks/plan.template.md`; its structure is `${CLAUDE_PLUGIN_ROOT}/playbooks/schemas/plan.schema.json`.

- You MUST write the plan from the template and run `anneal lint plan <file>` until it exits 0, applying each finding's `fix`.
- You MUST give every task its `Files:`, one `Test:` command and its `Blocked by:` line; `anneal lint plan` rejects a task without them, a blocker that is not a task, and a cycle.
- You MUST run `anneal plan approve <file>` only after the human approves the plan you showed them; it refuses while the spec has any `[NEEDS CLARIFICATION]` marker.
- You MUST run `anneal plan check <file>` before building from a plan; exit 4 means it changed since approval, so it goes back to the human.

## When to use

- A spec passes `anneal lint spec` with `data.needs_clarification` at 0, and it has no approved plan yet.
- Not when the request is new or vague (use `grill`), nor when the spec is unwritten (use `spec`), nor when an approved task is next (use `build`).

## Process

1. **Read the spec.** Read it whole, and the code its requirements touch. It's working if you can name the files each acceptance criterion changes.
2. **Slice.** Cut tracer-bullet tasks: each one a thin path through every layer, ending in a test that passes (references/slice.md). It's working if every task ends green on its own.
3. **Write each task.** `### T<n>: <title>`, its three lines, then test-first steps: failing test, run it red, code, run it green. It's working if a session with only this task and the spec could build it.
4. **Block sparingly.** Add a blocker only where a task needs another's output. It's working if most tasks can run in parallel.
5. **Write the Review focus.** Where a reviewer should look hardest, and why. It's working if each item names a file or a task.
6. **Lint.** Run `anneal lint plan docs/plans/<feature>.md` and apply each fix. It's working if it exits 0.
7. **Approve.** Show the human the plan; on their explicit yes, run `anneal plan approve`. It's working if it exits 0.
8. **Keep it current.** Any edit voids the approval; on exit 4 from `anneal plan check`, show the change and approve again. It's working if the check exits 0 when building starts.

## Rationalizations

| Excuse | Reality |
|---|---|
| "The tasks are obvious from the spec" | Then writing them down is quick, and the review sees them. |
| "One big task is simpler" | One big task has no tracer bullet and no early green. |
| "I'll pick the test command later" | A task without a command cannot prove it is done. |
| "Everything depends on everything" | Then the slices are horizontal layers. Cut through them. |
| "A small edit doesn't need re-approval" | The hash changed; `plan check` exits 4. Ask again. |

## Red flags

- Approving while the spec still has a `[NEEDS CLARIFICATION]` marker.
- A task whose files are "various" or whose test is "run the tests".
- Tasks cut by layer (all models, then all handlers) instead of by path.
- Editing the plan after approval without running `anneal plan check`.

## Verification

- `anneal lint plan docs/plans/<feature>.md` exits 0.
- `anneal plan check docs/plans/<feature>.md` exits 0.
- `anneal ledger query --type plan_approved` shows the plan's path and its `sha256:` hash.
