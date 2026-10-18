---
id: "kb-decision-wi-106144"
title: "Keep exit 3"
kind: decision
status: accepted
authority_tier: decision
summary: "In the context of \"Should dag check --plan exit 3 when tickets disagree with the plan (1 = cycle, 2 = dangling stay)?\", facing dag uses 1 for a cycle and 2 for a dangling id; a plan disagreement needed its own code, we decided for Keep exit 3 and neglected Reuse exit 1, on the premise that callers distinguish cycle from disagreement, accepting that if wrong: one more code to document."
depends_on: []
related: []
source_paths: []
last_reviewed: "2026-10-18"
reversibility: high
phase: null
supersedes: []
superseded_by: null
weighin_item: "wi-106144"
question: "Should dag check --plan exit 3 when tickets disagree with the plan (1 = cycle, 2 = dangling stay)?"
door: two-way
blast_radius: external
decider: "human:ryan"
provenance: user-approved
decided_at: "2026-10-18T21:10:54Z"
tree_hash: "0f26f9d8d3d80de80ec7b1cb4a5069c619ffbf45"
recommended: "A"
flip_condition: "you want one findings code per command"
---

# Keep exit 3

## Context and problem statement

Should dag check --plan exit 3 when tickets disagree with the plan (1 = cycle, 2 = dangling stay)?

dag uses 1 for a cycle and 2 for a dangling id; a plan disagreement needed its own code.

Raised by an agent (sweep) as a call and captured by Weigh-In as `wi-106144`. Anchor: `crates/anneal-cli/src/command.rs:70`.

## Decision drivers

- Change now

## Considered options

### A · Keep exit 3 (chosen)

Disagreement exits 3

- Holds if callers distinguish cycle from disagreement.
- If wrong: one more code to document.

### B · Reuse exit 1

Disagreement shares the findings code with cycles

- Holds if callers only check zero vs non-zero.
- If wrong: a cycle and a disagreement look the same.

## Evidence

- `crates/anneal-cli/src/command.rs:70`: DAG_PROVES ... every ticket is a task of the plan or an outside blocker of one

## Decision outcome

Chosen option: **Keep exit 3**, the recommended option.

Decider's note: Ryan, card 2026-10-18: keep exit 3 for a plan disagreement

### Consequences

- Good, because it holds if callers distinguish cycle from disagreement.
- Bad, because if wrong: one more code to document.

### Confirmation

Revisit when: you want one findings code per command

## Why this might be wrong

if you standardize on 1 for all findings

## Provenance

- Decided 2026-10-18T21:10:54Z by human:ryan (user-approved), via chat.
- Raised in session `8be558d7-9d48-52d4-b638-cb5f9847aeb0`.
- Verbatim: "Should dag check --plan exit 3 when tickets disagree with the plan (1 = cycle, 2 = dangling stay)?"
