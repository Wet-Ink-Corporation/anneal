---
id: "kb-decision-wi-df4670"
title: "files: only"
kind: decision
status: accepted
authority_tier: decision
summary: "In the context of \"Should the dag estimate count only the tracker's files: field for markdown tickets (rest of #68), changing a pinned estimate test?\", facing markdown tickets list files: explicitly; counting prose paths too inflates estimates, we decided for files: only and neglected Keep both, on the premise that trackers fill files: reliably, accepting that if wrong: tickets without files: lose their estimate."
depends_on: []
related: []
source_paths: []
last_reviewed: "2026-10-18"
reversibility: high
phase: null
supersedes: []
superseded_by: null
weighin_item: "wi-df4670"
question: "Should the dag estimate count only the tracker's files: field for markdown tickets (rest of #68), changing a pinned estimate test?"
door: two-way
blast_radius: module
decider: "human:ryan"
provenance: user-approved
decided_at: "2026-10-18T21:10:54Z"
tree_hash: "0f26f9d8d3d80de80ec7b1cb4a5069c619ffbf45"
recommended: "A"
flip_condition: "files: is often left empty"
---

# files: only

## Context and problem statement

Should the dag estimate count only the tracker's files: field for markdown tickets (rest of #68), changing a pinned estimate test?

Markdown tickets list files: explicitly; counting prose paths too inflates estimates.

Raised by an agent (sweep) as a question and captured by Weigh-In as `wi-df4670`. Anchor: `crates/anneal-cli/src/dag/estimate.rs:84`.

## Decision drivers

- Test change

## Considered options

### A · files: only (chosen)

Count only the files: field when a ticket has one; change the pinned test

- Holds if trackers fill files: reliably.
- If wrong: tickets without files: lose their estimate.

### B · Keep both

Keep counting body paths as well

- Holds if files: is often incomplete.
- If wrong: estimates stay inflated.

## Evidence

- `crates/anneal-cli/src/dag/estimate.rs:84`: its listed files, then backticked paths in its body

## Decision outcome

Chosen option: **files: only**, the recommended option.

Decider's note: Ryan, card 2026-10-18: count only the tracker's files: field for markdown tickets

### Consequences

- Good, because it holds if trackers fill files: reliably.
- Bad, because if wrong: tickets without files: lose their estimate.

### Confirmation

Revisit when: files: is often left empty

## Why this might be wrong

if body paths carry files the tracker omits

## Provenance

- Decided 2026-10-18T21:10:54Z by human:ryan (user-approved), via chat.
- Raised in session `8be558d7-9d48-52d4-b638-cb5f9847aeb0`.
- Verbatim: "Should the dag estimate count only the tracker's files: field for markdown tickets (rest of #68), changing a pinned estimate test?"
