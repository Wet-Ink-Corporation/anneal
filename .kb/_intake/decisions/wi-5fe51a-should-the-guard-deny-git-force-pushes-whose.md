---
id: "kb-decision-wi-5fe51a"
title: "Deny expansions too"
kind: decision
status: accepted
authority_tier: decision
summary: "In the context of \"Should the guard deny git force pushes whose target comes from an expansion such as git push \"$branch\" (A), or only document the gap (C)?\", facing the guard allows git push \"$branch\" with a force flag hidden in the expansion, we decided for Deny expansions too and neglected Document the gap, on the premise that agents rarely push to a variable branch, accepting that if wrong: legitimate scripted pushes get denied."
depends_on: []
related: []
source_paths: []
last_reviewed: "2026-10-18"
reversibility: high
phase: null
supersedes: []
superseded_by: null
weighin_item: "wi-5fe51a"
question: "Should the guard deny git force pushes whose target comes from an expansion such as git push \"$branch\" (A), or only document the gap (C)?"
door: two-way
blast_radius: system
decider: "human:ryan"
provenance: user-approved
decided_at: "2026-10-18T21:10:29Z"
tree_hash: "0f26f9d8d3d80de80ec7b1cb4a5069c619ffbf45"
recommended: "A"
flip_condition: "your agents push to variable branches often"
---

# Deny expansions too

## Context and problem statement

Should the guard deny git force pushes whose target comes from an expansion such as git push "$branch" (A), or only document the gap (C)?

The guard allows git push "$branch" with a force flag hidden in the expansion.

Raised by an agent (sweep) as a question and captured by Weigh-In as `wi-5fe51a`. Anchor: `crates/anneal-cli/src/hook/guard/denylist/readings.rs:1`.

## Decision drivers

- False denials

## Considered options

### A · Deny expansions too (chosen)

Treat a dynamic push target as possibly forced and deny

- Holds if agents rarely push to a variable branch.
- If wrong: legitimate scripted pushes get denied.

### C · Document the gap

Keep allowing it; list it in docs/guard.md

- Holds if the risk is acceptable.
- If wrong: a forced push slips through.

## Evidence

- `crates/anneal-cli/src/hook/guard/denylist/readings.rs:1`: expansions are read as written unless brace-built

## Decision outcome

Chosen option: **Deny expansions too**, the recommended option.

Decider's note: Ryan, card 2026-10-18: deny force pushes whose target comes from an expansion too

### Consequences

- Good, because it holds if agents rarely push to a variable branch.
- Bad, because if wrong: legitimate scripted pushes get denied.

### Confirmation

Revisit when: your agents push to variable branches often

## Why this might be wrong

if variable-target pushes are common in your workflows

## Provenance

- Decided 2026-10-18T21:10:29Z by human:ryan (user-approved), via chat.
- Raised in session `8be558d7-9d48-52d4-b638-cb5f9847aeb0`.
- Verbatim: "Should the guard deny git force pushes whose target comes from an expansion such as git push \"$branch\" (A), or only document the gap (C)?"
