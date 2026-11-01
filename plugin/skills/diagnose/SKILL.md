---
name: diagnose
description: Use when told to fix a bug or that X should do Y but does Z, something broken or a test failing unexpectedly, even if rushed or trivial; reproduce it red, then find the root cause before fixing it. Do NOT use for a vague request (use `grill`) or an approved task (use `build`).
---

# Diagnose

## Overview

Finds a failure's root cause from evidence, starting from a loop that shows it on demand. In `anneal mode diagnose` the `preconditions` hook refuses source edits until a red reproduction is recorded.

- You MUST record a failing reproduction with `anneal repro record --id <bug> -- <command>` before changing source; only a non-zero exit is recorded, and exit 3 means the command passed, so it reproduces nothing.
- You MUST tag every temporary log line, print or probe with `[DEBUG-<id>]`, and run `anneal repro leftovers` until it exits 0 before you finish.
- You MUST run `anneal task done <bug> -- <command>` before reporting, the reproduction as `<command>` if no test is wanted; the `completeness` hook blocks a fixed claim without it.
- You MUST NOT route around a `preconditions` denial (writing source through Bash, say); record the reproduction instead.

## When to use

- A test fails, a command errors or a behaviour is wrong, and the cause is not yet known.
- Not for a vague request (use `grill`) or an approved task (use `build`).

## Process

1. **Build the loop.** Find the fastest command that shows the failure, preferably a test, which stays as the regression test. It's working if two runs fail the same way.
2. **Record it red.** Run `anneal mode diagnose`, then `anneal repro record --id <bug> -- <command>`. It's working if it exits 0 with `data.recorded` true.
3. **Read the evidence.** Read the whole error, the trace and recent changes (`git log`, `git diff`), and compare a nearby case that works. It's working if you can say where the paths part.
4. **Rank hypotheses.** List three to five causes, most likely first. It's working if each names the observation that would rule it out.
5. **Test one at a time.** Change one thing, or add tagged instrumentation, then run the loop and drop what it refutes. It's working if every run removes or confirms a hypothesis.
6. **Fix the cause.** One change at the cause, not the symptom; after three failed fixes, question the design with the human. It's working if the loop and the rest of the suite are green.
7. **Clean up.** Remove tagged lines; keep the reproduction as a test. It's working if `anneal repro leftovers` exits 0.
8. **Report.** Run `anneal task done`, then report the cause, the hypotheses ruled out, the fix and that run. It's working if it exits 0.

## Rationalizations

| Excuse | Reality |
|---|---|
| "I can see the bug, I'll just fix it" | Without a red loop, nothing shows the fix worked. |
| "It's too hard to reproduce" | Then the first job is a narrower loop, not a guess. |
| "One more fix will do it" | Repeated failed fixes mean the model of the code is wrong; go back to the evidence. |
| "The debug prints are harmless" | Untagged prints ship. Tag them so the scan finds them. |
| "It passes now, so it was flaky" | A failure that vanished unexplained comes back. |
| "The lead said skip the tests" | The reproduction is the test. Run `task done` with it. |

## Red flags

- Editing source before `anneal repro record` has exited 0.
- Changing several things in one run, so nobody knows which mattered.
- A fix that hides the symptom: a catch, a retry, a skipped test.

## Verification

- `anneal ledger query --type repro_red` shows the reproduction, with this worktree.
- The reproduction passes now and stays in the test suite.
