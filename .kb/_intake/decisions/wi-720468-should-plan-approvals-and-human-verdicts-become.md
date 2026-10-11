---
id: "kb-decision-wi-720468"
title: "Model may approve, provenance recorded"
kind: decision
status: accepted
authority_tier: decision
summary: "In the context of \"Should plan approvals and human verdicts become human-only, with the guard denying model-issued anneal plan approve and verdict set --actor human?\", facing today the model can run anneal plan approve itself, so an approved plan proves nothing about a human, we decided for Model may approve, provenance recorded and neglected Human-only approvals; Status quo, on the premise that AFK plan approval matters more, accepting that if wrong: a plan gate the model can satisfy alone."
depends_on: []
related: []
source_paths: []
last_reviewed: "2026-10-11"
reversibility: low
phase: null
supersedes: []
superseded_by: null
weighin_item: "wi-720468"
question: "Should plan approvals and human verdicts become human-only, with the guard denying model-issued anneal plan approve and verdict set --actor human?"
door: one-way
blast_radius: system
decider: "human:ryan"
provenance: user-approved
decided_at: "2026-10-11T15:41:14Z"
tree_hash: "955c5d1766dc0040a710a71bbc7e062086329e95"
recommended: "B"
flip_condition: "Ryan wants plan approvals to be human-only too"
---

# Model may approve, provenance recorded

## Context and problem statement

Should plan approvals and human verdicts become human-only, with the guard denying model-issued anneal plan approve and verdict set --actor human?

Today the model can run anneal plan approve itself, so an approved plan proves nothing about a human.

Raised by an agent (sweep) as a call and captured by Weigh-In as `wi-720468`. Anchor: `https://github.com/Wet-Ink-Corporation/anneal/issues/62`.

## Decision drivers

- AFK runs
- Gate integrity

## Considered options

### A · Human-only approvals

Guard denies model-issued approve and human verdicts; people use /anneal or their terminal

- Holds if approval must mean a person looked.
- If wrong: AFK runs stall at every plan approval.

### B · Model may approve, provenance recorded (chosen)

Keep it allowed; ledger records actor model vs human, merge-gate requires human only for verdicts

- Holds if AFK plan approval matters more.
- If wrong: a plan gate the model can satisfy alone.

### C · Status quo

No denylist entry, no provenance

- Holds if approvals are advisory.
- If wrong: the gate is honor-system, the thing anneal exists to fix.

## Evidence

- `https://github.com/Wet-Ink-Corporation/anneal/issues/62`: guard denies model-issued anneal plan approve and verdict set --actor human
- `memory:anneal-github-backlog`: a human approved verdict stands in for the review plugin until it ships

## Decision outcome

Chosen option: **Model may approve, provenance recorded**, the recommended option.

Decider's note: Chosen on the decision card: model may approve plans (actor recorded); human verdicts stay human-only

### Consequences

- Good, because it holds if AFK plan approval matters more.
- Bad, because if wrong: a plan gate the model can satisfy alone.

### Confirmation

Revisit when: Ryan wants plan approvals to be human-only too

## Why this might be wrong

If AFK runs are rare after v0.1, A is cleaner

## Provenance

- Decided 2026-10-11T15:41:14Z by human:ryan (user-approved), via cli.
- Raised in session `79d77b96-1e25-5b54-bd19-6d40243f7bdd`.
- Verbatim: "Should plan approvals and human verdicts become human-only, with the guard denying model-issued anneal plan approve and verdict set --actor human?"
