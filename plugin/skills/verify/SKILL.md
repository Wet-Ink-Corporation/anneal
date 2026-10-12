---
name: verify
description: Use when work is about to be called done, fixed or passing, even if the tests ran earlier; prove it with a fresh test run and a doubt pass first. Do NOT use for a failure whose cause is unknown (use `diagnose`), splitting a spec into tasks (use `plan`) or opening the pull request (use `ship`).
---

# Verify

## Overview

Verification comes before completion: "done" or "passing" in a reply needs fresh evidence in the ledger, not the author's word. The `completeness` hook checks each reply when the session stops.

- You MUST run the tests with `anneal task done <id> -- <test command>` after the last source edit and before writing done, fixed or passing; the `completeness` hook blocks the claim until a green run is newer than the edit.
- You MUST remove every `[DEBUG-<id>]` tag until `anneal repro leftovers` exits 0; the `completeness` hook blocks while one is left.
- You MUST end every doubt-pass reviewer's reply with a Status block (`Status:` DONE, DONE_WITH_CONCERNS, NEEDS_CONTEXT or BLOCKED, then Gate, Review, Changed, Decisions needed, Not verified); the `status` hook records each one.

## When to use

- A task, a fix or a change looks finished and you are about to say so.
- Not for a failure nobody understands yet (use `diagnose`), nor for planning (use `plan`). The pull request and its review are `ship`'s and the review plugin's: this skill stops at the local pass.

## Process

1. **Run the tests fresh.** Run `anneal task done <id> -- <test command>` with the task's own test command. It's working if it exits 0 and `anneal ledger tail` shows a green `test_run` after your last edit.
2. **Run the gate.** In a Rust repository (a `Cargo.toml` at the root), run `temper:gate`. It's working if the gate is green.
3. **Clear the leftovers.** Run `anneal repro leftovers`. It's working if it exits 0.
4. **Doubt pass.** Hand one or more reviewer subagents the artifact (the diff and the files it touches) and the contract (the spec's criteria, the task's test, the plan), never your claim or summary. Ask each for findings as `file:line`, what is wrong and why, ending in a Status block. It's working if each reviewer got files, not your conclusions.
5. **Triage.** Reject every finding without a `file:line`. Fix each one you accept, then go back to step 1. It's working if each cycle ends with no accepted finding, within 3 cycles; after the third, stop and hand the open findings to the human.
6. **Report.** Claim only what the ledger shows: name the green `test_run`, the gate result and any finding left open. It's working if every claim points to evidence.

## Rationalizations

| Excuse | Reality |
|---|---|
| "The tests passed an hour ago" | An edit since then makes that run stale. Run them again. |
| "I'll tell the reviewer what I changed, it saves time" | A reviewer told the conclusion looks for it. Give the diff and the contract. |
| "That finding is vague but probably right" | Without `file:line` nobody can check it. Reject it. |
| "One more doubt cycle will settle it" | Three cycles without agreement is a question for the human. |
| "CI will catch it" | CI runs after the claim. The claim needs evidence now. |

## Red flags

- "Done" or "all tests pass" with no `test_run` in the ledger since the last edit.
- A reviewer prompt that contains your summary or says the work is finished.
- A finding accepted, or argued over, without a `file:line`.
- A fourth doubt cycle.

## Verification

- `anneal ledger tail` shows a green `test_run` newer than the last source change.
- `anneal repro leftovers` exits 0, and in a Rust repository `temper:gate` is green.
- `anneal ledger query --type subagent_status` shows each reviewer's status.
