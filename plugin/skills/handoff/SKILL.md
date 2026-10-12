---
name: handoff
description: Use at a phase boundary or when the context monitor warns, even if the context still seems roomy; choose continue, clear, hand off, a subagent or compact, and put what must survive on disk. Do NOT use for proving work done (use `verify`) or splitting work into tasks (use `plan`).
disable-model-invocation: true
argument-hint: "[what the next session must know]"
---

# Handoff

## Overview

Whatever must outlive `/clear` or `/compact` lives on disk, and a script writes it. The `handoff` hook writes `.continue-here` at the repository root before every compaction, and the `bootstrap` hook injects it at the next session start.

- You MUST write the note with `anneal hook handoff` before a `/clear` that leaves work unfinished; the `bootstrap` hook injects only what is on disk.
- You MUST stop and hand off when the `context-monitor` hook stops you at 75% of the auto-compact window.

## When to use

- A phase ends (spec approved, task done, review settled) and the next one starts.
- The `context-monitor` hook warned at 60% or stopped you at 75%.
- Not to claim the work is finished (use `verify`), nor to split it (use `plan`).

## The choice

| Situation | Choice | How |
|---|---|---|
| Same phase, below the 60% warning | continue | Carry on. |
| Phase finished; the next needs nothing said in this conversation | clear | `/clear`; the bootstrap re-injects the index and the ledger. |
| Phase unfinished; past the warning, or stopped by the monitor | hand off | Write the note, then `/clear`. |
| The next piece stands alone (a search, a review, one task) | subagent | Give it files and the contract, never your summary. |
| Phase unfinished and this conversation's detail is still needed | compact | `/compact <what to keep>`; the hook writes the note first. |

## Process

1. **Choose.** Take the first row of the table that fits. It's working if you can name the row.
2. **Write the facts.** For hand off or compact, run `anneal hook handoff` with `{}` on stdin (compaction runs it for you). It's working if `.continue-here` shows the phase, the task N of M, the open gates and the ledger line.
3. **Write the notes.** Under `## Notes` in `.continue-here`, add what a script cannot know: decisions in flight, what was tried and failed, the next step, and any argument given to this skill. It's working if a stranger could take the next step from the file alone.
4. **Act.** Run `/clear` or `/compact`, start the subagent, or continue. It's working if the next session's start shows the note.
5. **Resume.** In the new session, read the injected note, check its open gates, and delete `.continue-here` once the work is picked up. It's working if no stale note is injected the session after.

## Rationalizations

| Excuse | Reality |
|---|---|
| "I'll remember where I was" | The next session starts with only what is on disk. |
| "The summary from /compact is enough" | A summary drops the open gates and the task count. The note keeps them. |
| "There is room left, I'll finish first" | Quality falls before the window is full; the thresholds allow for that. |
| "The subagent needs my reasoning" | It needs the files and the contract, or it checks your conclusion instead of the work. |

## Red flags

- `/clear` with unfinished work and no `.continue-here`.
- Ignoring a stop from the `context-monitor` hook to squeeze in one more step.
- Notes that repeat the facts above them instead of the decisions behind them.
- A note days old injected into an unrelated session.

## Verification

- `.continue-here` exists at the repository root and its facts match `anneal ledger tail`.
- The next session's start shows the note after the index.
- `.continue-here` is gone once the work it describes is picked up.
