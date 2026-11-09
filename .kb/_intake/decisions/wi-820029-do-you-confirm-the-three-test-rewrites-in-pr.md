---
id: "kb-decision-wi-820029"
title: "Confirm, merge"
kind: decision
status: accepted
authority_tier: decision
summary: "In the context of \"Do you confirm the three test rewrites in PR 235 (guard Clef tests via judge_with, renamed grill classify fallback test, lint fix-text expectations) so it can merge?\", facing h-16: test rewrites are human decisions, we decided for Confirm, merge and neglected Review first, on the premise that rewrites follow from D2, accepting that if wrong: a fallback path loses coverage."
depends_on: []
related: []
source_paths: []
last_reviewed: "2026-11-09"
reversibility: high
phase: null
supersedes: []
superseded_by: null
weighin_item: "wi-820029"
question: "Do you confirm the three test rewrites in PR 235 (guard Clef tests via judge_with, renamed grill classify fallback test, lint fix-text expectations) so it can merge?"
door: two-way
blast_radius: module
decider: "human:claude"
provenance: user-approved
decided_at: "2026-11-09T00:30:16Z"
tree_hash: "bdbdfcc43748de53098aa8e7cd83224cf7d36b06"
recommended: "A"
flip_condition: "a rewrite weakens an assertion"
---

# Confirm, merge

## Context and problem statement

Do you confirm the three test rewrites in PR 235 (guard Clef tests via judge_with, renamed grill classify fallback test, lint fix-text expectations) so it can merge?

H-16: test rewrites are human decisions.

Raised by an agent (sweep) as a question and captured by Weigh-In as `wi-820029`. Anchor: `crates/anneal-cli/src/hook/guard/clef/tests.rs:1`.

## Decision drivers

- speed

## Considered options

### A · Confirm, merge (chosen)

merge PR 235, then open wave 2a

- Holds if rewrites follow from D2.
- If wrong: a fallback path loses coverage.

### B · Review first

hold until Ryan reads them

- Holds if Ryan wants to read the diff.
- If wrong: waves stay queued.

## Evidence

- `https://github.com/Wet-Ink-Corporation/anneal/pull/235`: three rewrites listed in the body

## Decision outcome

Chosen option: **Confirm, merge**, the recommended option.

Decider's note: Ryan tapped A on the card

### Consequences

- Good, because it holds if rewrites follow from D2.
- Bad, because if wrong: a fallback path loses coverage.

### Confirmation

Revisit when: a rewrite weakens an assertion

## Why this might be wrong

the reviewer, not Ryan, judged the rewrites

## Provenance

- Decided 2026-11-09T00:30:16Z by human:claude (user-approved), via chat.
- Raised in session `ce701b16-9785-59d6-aa30-13e1f5f1c96d`.
- Verbatim: "Do you confirm the three test rewrites in PR 235 (guard Clef tests via judge_with, renamed grill classify fallback test, lint fix-text expectations) so it can merge?"
