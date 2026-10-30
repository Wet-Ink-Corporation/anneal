---
id: "kb-decision-wi-d75b26"
title: "No bump, wording only"
kind: decision
status: accepted
authority_tier: decision
summary: "In the context of \"Should #77 bump the ledger registry version?\", facing #77 asks whether the ledger registry version moves; that is a persisted contract other readers check, we decided for No bump, wording only and neglected Bump the registry version, on the premise that no event's meaning changed, accepting that if wrong: old ledgers read with new meaning."
depends_on: []
related: []
source_paths: []
last_reviewed: "2026-10-18"
reversibility: low
phase: null
supersedes: []
superseded_by: null
weighin_item: "wi-d75b26"
question: "Should #77 bump the ledger registry version?"
door: one-way
blast_radius: system
decider: "human:ryan"
provenance: user-approved
decided_at: "2026-10-18T21:10:29Z"
tree_hash: "0f26f9d8d3d80de80ec7b1cb4a5069c619ffbf45"
recommended: "B"
flip_condition: "any event field's meaning changed in #76"
---

# No bump, wording only

## Context and problem statement

Should #77 bump the ledger registry version?

#77 asks whether the ledger registry version moves; that is a persisted contract other readers check.

Raised by an agent (sweep) as a deferral and captured by Weigh-In as `wi-d75b26`. Anchor: `crates/anneal-ledger/src/envelope.rs:1`.

## Decision drivers

- Migration work

## Considered options

### A · Bump the registry version

Ship #77 as a registry bump with a reader for the old version

- Holds if readers must tell old and new events apart.
- If wrong: an extra migration path to maintain.

### B · No bump, wording only (chosen)

Close #77 as done by #76's wording change

- Holds if no event's meaning changed.
- If wrong: old ledgers read with new meaning.

## Evidence

- `crates/anneal-ledger/src/envelope.rs:115`: envelope fields are tagged per event kind

## Decision outcome

Chosen option: **No bump, wording only**, the recommended option.

Decider's note: Ryan, card 2026-10-18: no bump, wording only

### Consequences

- Good, because it holds if no event's meaning changed.
- Bad, because if wrong: old ledgers read with new meaning.

### Confirmation

Revisit when: any event field's meaning changed in #76

## Why this might be wrong

if a downstream reader needs to know the plan-check wording era

## Provenance

- Decided 2026-10-18T21:10:29Z by human:ryan (user-approved), via chat.
- Raised in session `8be558d7-9d48-52d4-b638-cb5f9847aeb0`.
- Verbatim: "Should #77 bump the ledger registry version?"
