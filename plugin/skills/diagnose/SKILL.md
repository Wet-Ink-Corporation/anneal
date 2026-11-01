---
name: diagnose
description: Use when told to fix a bug or that X should do Y but does Z, something broken or a test failing unexpectedly, even if rushed or trivial; reproduce it red, then find the root cause before fixing it. Do NOT use for a vague request (use `grill`) or an approved task (use `build`).
---

# Diagnose

## Overview

Finds a failure's root cause from a loop that shows it on demand. In `anneal mode diagnose` the `preconditions` hook refuses source edits until a red reproduction is recorded.

- You MUST record a failing reproduction with `anneal repro record --id <bug> -- <command>` before changing source; exit 3 means it passed and reproduces nothing.
- You MUST tag every temporary probe `[DEBUG-<id>]` and run `anneal repro leftovers` until it exits 0.
- You MUST run `anneal task done <bug> -- <command>` before reporting a fix, the reproduction as `<command>` if no test is wanted; the `completeness` hook blocks a fixed claim without it.
- You MUST NOT route around a `preconditions` denial (writing source through Bash, say); record the reproduction instead.

## When to use

- A test fails, a command errors or a behaviour is wrong, and the cause is not yet known.

## Process

1. **Build the loop.** Find the fastest command that shows the failure, ideally a test you keep. If none can run here (the fix is already in, a tool is blocked), say so, fix nothing and go to step 8. It's working if two runs fail the same way, or you said why none can.
2. **Record it red.** Run `anneal mode diagnose`, then `anneal repro record --id <bug> -- <command>`. It's working if it exits 0 with `data.recorded` true.
3. **Read the evidence.** Read the whole error, the trace and recent changes (`git log`, `git diff`); compare a working case. It's working if you can say where the paths part.
4. **Rank hypotheses.** List three to five causes, most likely first. It's working if each names the observation that would rule it out.
5. **Test one at a time.** Change one thing or add a tagged probe, run the loop, drop what it refutes. It's working if every run removes or confirms a hypothesis.
6. **Fix the cause.** One change at the cause, not the symptom; after three failed fixes, ask the human about the design. It's working if the loop and the suite are green.
7. **Clean up.** Remove tagged lines; keep the reproduction as a test. It's working if `anneal repro leftovers` exits 0.
8. **Report.** With a fix, run `anneal task done <bug> -- <command>`, then lead with the reproduction (red before the edit, green after), then the cause, ruled-out hypotheses and fix. With no loop, report the ranked hypotheses, each with the check that would rule it out, and no fix. It's working if the report opens with the reproduction or the list.

## Rationalizations

| Excuse | Reality |
|---|---|
| "I can see the bug, I'll just fix it" | Without a red loop, nothing shows the fix worked. |
| "It's too hard to reproduce" | Try a narrower loop; if none runs, rank hypotheses instead of guessing. |
| "One more fix will do it" | Repeated failed fixes mean your model is wrong. |
| "The debug prints are harmless" | Untagged prints ship. |
| "It passes now, so it was flaky" | A failure that vanished unexplained comes back. |
| "The lead said skip the tests" | The reproduction is the test. Run `task done` with it. |

## Red flags

- Editing source before `anneal repro record` has exited 0.
- Changing several things in one run.
- A fix that hides the symptom: a catch, a retry, a skipped test.

## Verification

- `anneal ledger query --type repro_red` shows the reproduction, with this worktree.
- The reproduction passes and stays in the suite.
