---
name: verify
description: Use when asked "is it fixed?", "tested?" or "can we close it?", or to check a claim, and before calling work done or fixed, even if told to skip tests; prove it fresh, failing before the fix and passing after, and doubt it. Do NOT use for an unknown failure (use `diagnose`) or the PR (use `ship`).
---

# Verify

## Overview

- You MUST run `anneal task done <id> -- <test command>` after the last source edit and before writing done, fixed or passing, and back any verdict with a test of the claim's case, red before any fix (a check by hand is not a test); the `completeness` hook blocks the claim until a green run is newer than the edit.
- You MUST remove every `[DEBUG-<id>]` tag until `anneal repro leftovers` exits 0; the `completeness` hook blocks while one is left.
- You MUST end every doubt-pass reviewer's reply with a Status block (`Status:` DONE, DONE_WITH_CONCERNS, NEEDS_CONTEXT or BLOCKED, then Gate, Review, Changed, Decisions needed, Not verified); the `status` hook records each one.

## When to use

- Work looks finished and you are about to say so.
- Not for an unknown failure (`diagnose`), planning (`plan`) or the pull request (`ship`).

## Process

1. **Find the contract.** Read the decision and spec rows the claim depends on (a decisions-log row in an `AGENTS.md`, say) and cite each by `file:line`. Search for the exact row; a broad `grep | head` can bury it. It's working if every row you judge against is cited.
2. **Run the tests fresh.** Run `anneal task done <id> -- <test command>`; `<id>` is the task id or a short name. A run that reached no test (`No module named pytest`) is not evidence; use the project's own runner (`python3 -m unittest`). The suite is not enough: run the claim's own case (for a cap, a file one past it), writing a repo test if none covers it; see it red before trying any patch anywhere. It's working if a fresh run after your last edit exercised the claim's case.
3. **Clear leftovers, run the gate.** Run `anneal repro leftovers`, and `temper:gate` in a Rust repository. It's working if both pass.
4. **Doubt pass.** Run `anneal verify risk` first; on exit 3, skip to step 6 and say so. Hand reviewer subagents the diff, its files and the contract (step 1's rows, the task's test, the plan), never your claim. Ask each for findings as `file:line`, what is wrong and why. It's working if each reviewer got files, not your conclusions.
5. **Triage.** Reject every finding without a `file:line`. Fix each one you accept, then go back to step 2. It's working if a cycle ends with no accepted finding within 3; after the third, hand the open findings to the human.
6. **Report.** Claim only what the ledger shows: name the green `test_run`, the gate result and any open finding. It's working if every claim points to evidence.

## Rationalizations

| Excuse | Reality |
|---|---|
| "The tests passed an hour ago" | An edit since makes it stale. |
| "I'll tell the reviewer what I changed" | A reviewer told the conclusion looks for it. |
| "CI will catch it" | CI runs after the claim. |
| "The lead said skip the tests" | Write and run the test anyway; a skip does not prove the claim. |
| "We ship in 5 minutes" | A run takes seconds. |
| "You only asked to verify" | Testing the claim is verifying, not fixing. |

## Red flags

- A "not fixed" or "not verified" argued from code, with no run of the case.

## Verification

- `anneal ledger tail` shows a green `test_run` newer than the last source change.
- `anneal repro leftovers` exits 0, and in a Rust repository `temper:gate` is green.
- `anneal ledger query --type subagent_status` shows each reviewer's status.
