---
name: diagnose
description: Use when something is broken or a test fails unexpectedly, even if the cause looks obvious; reproduce it red, then find the root cause before fixing it. Do NOT use for a new or vague request (use `grill`), splitting a spec into tasks (use `plan`) or building an approved task (that is `build`).
---

# Diagnose

## Overview

Finds the root cause of a failure from evidence, starting from a loop that shows the failure on demand. Enter diagnose mode with `anneal mode diagnose`; the `preconditions` hook then refuses source edits in this worktree until a red reproduction is recorded there.

- You MUST record a failing reproduction with `anneal repro record --id <bug> -- <command>` before changing source; only a non-zero exit is recorded, and exit 3 means the command passed, so it reproduces nothing.
- You MUST tag every temporary log line, print or probe with `[DEBUG-<id>]`, and run `anneal repro leftovers` until it exits 0 before you finish.
- You MUST NOT route around a `preconditions` denial (writing source through Bash, say); record the reproduction instead.

## When to use

- A test fails, a command errors or a behaviour is wrong, and the cause is not yet known.
- Not for a new or vague request (use `grill`), nor for planned work on an approved task (that is `build`).

## Process

1. **Build the loop.** Find the fastest command that shows the failure: a failing test, a script, a request. Prefer a test, since it stays as the regression test. It's working if two runs fail the same way.
2. **Record it red.** Run `anneal mode diagnose`, then `anneal repro record --id <bug> -- <command>`. It's working if it exits 0 with `data.recorded` true.
3. **Read the evidence.** Read the whole error, the stack trace and the recent changes (`git log`, `git diff`) before guessing, and compare with a nearby case that works. It's working if you can say where the working and failing paths part.
4. **Rank hypotheses.** Write three to five causes, most likely first, each predicting something the loop can show. It's working if each one names the observation that would rule it out.
5. **Test one at a time.** Change one thing, or add tagged instrumentation, then run the loop and drop what it refutes. It's working if every run removes or confirms a hypothesis.
6. **Fix the cause.** Make the reproduction pass with one change at the cause, not at the symptom. After three failed fixes, stop and question the design with the human. It's working if the loop and the rest of the suite are green.
7. **Clean up.** Remove every tagged line and keep the reproduction as a regression test. It's working if `anneal repro leftovers` exits 0.

## Rationalizations

| Excuse | Reality |
|---|---|
| "I can see the bug, I'll just fix it" | Without a red loop, nothing shows the fix worked. |
| "It's too hard to reproduce" | Then the first job is a narrower loop, not a guess. |
| "One more fix will do it" | Repeated failed fixes mean the model of the code is wrong; go back to the evidence. |
| "The debug prints are harmless" | Untagged prints ship. Tag them so the scan finds them. |
| "It passes now, so it was flaky" | A failure that vanished unexplained comes back. |

## Red flags

- Editing source before `anneal repro record` has exited 0.
- Changing several things in one run, so nobody knows which mattered.
- A fix that hides the symptom: a catch, a retry, a skipped test.
- Instrumentation without a `[DEBUG-<id>]` tag.

## Verification

- `anneal ledger query --type repro_red` shows the reproduction, with this worktree.
- The reproduction passes now and stays in the test suite.
- `anneal repro leftovers` exits 0.
