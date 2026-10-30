---
id: "kb-decision-wi-5fe51a"
title: "Document the gap"
kind: decision
status: accepted
authority_tier: decision
summary: "In the context of \"Should the guard deny git force pushes whose target comes from an expansion such as git push \"$branch\" (A), or only document the gap (C)?\", facing the guard allows git push \"$branch\" with a force flag hidden in the expansion, we decided for Document the gap and neglected Deny expansions too, on the premise that the risk is acceptable, accepting that if wrong: a forced push slips through."
depends_on: []
related: []
source_paths: []
last_reviewed: "2026-10-19"
reversibility: high
phase: null
supersedes: []
superseded_by: null
weighin_item: "wi-5fe51a"
question: "Should the guard deny git force pushes whose target comes from an expansion such as git push \"$branch\" (A), or only document the gap (C)?"
door: two-way
blast_radius: system
decider: "human:ryan"
provenance: user-directed
decided_at: "2026-10-19T01:17:00Z"
tree_hash: "a09d3a0debb1f365c3af1735a1c335b9038942c1"
recommended: "A"
flip_condition: "your agents push to variable branches often"
---

# Document the gap

## Context and problem statement

Should the guard deny git force pushes whose target comes from an expansion such as git push "$branch" (A), or only document the gap (C)?

The guard allows git push "$branch" with a force flag hidden in the expansion.

Raised by an agent (sweep) as a question and captured by Weigh-In as `wi-5fe51a`. Anchor: `crates/anneal-cli/src/hook/guard/denylist/readings.rs:1`.

## Decision drivers

- False denials

## Considered options

### A · Deny expansions too

Treat a dynamic push target as possibly forced and deny

- Holds if agents rarely push to a variable branch.
- If wrong: legitimate scripted pushes get denied.

### C · Document the gap (chosen)

Keep allowing it; list it in docs/guard.md

- Holds if the risk is acceptable.
- If wrong: a forced push slips through.

## Evidence

- `crates/anneal-cli/src/hook/guard/denylist/readings.rs:1`: expansions are read as written unless brace-built

## Decision outcome

Chosen option: **Document the gap**, overriding the recommendation (A).

Decider's note: Ryan, card 2026-10-19: document the gap; supersedes his earlier A, matching his #127 walkthrough pick

### Consequences

- Good, because it holds if the risk is acceptable.
- Bad, because if wrong: a forced push slips through.

### Confirmation

Revisit when: your agents push to variable branches often

## Why this might be wrong

if variable-target pushes are common in your workflows

## Provenance

- Decided 2026-10-19T01:17:00Z by human:ryan (user-directed), via chat.
- Raised in session `8be558d7-9d48-52d4-b638-cb5f9847aeb0`.
- Verbatim: "Should the guard deny git force pushes whose target comes from an expansion such as git push \"$branch\" (A), or only document the gap (C)?"
