---
id: "kb-decision-wi-785d28"
title: "Fold into dogfood"
kind: decision
status: accepted
authority_tier: decision
summary: "In the context of \"Should #6's live-orchestrate exit criterion be met by folding a 3+ ticket /anneal:orchestrate run into the temper dogfood plus the real weigh-in park test, with the criterion reworded?\", facing #6 asks for a live D2 park, which shadow mode cannot produce, we decided for Fold into dogfood and neglected Separate run; Tests only, on the premise that one live session is enough evidence, accepting that if wrong: a second live run later."
depends_on: []
related: []
source_paths: []
last_reviewed: "2026-10-19"
reversibility: high
phase: null
supersedes: []
superseded_by: null
weighin_item: "wi-785d28"
question: "Should #6's live-orchestrate exit criterion be met by folding a 3+ ticket /anneal:orchestrate run into the temper dogfood plus the real weigh-in park test, with the criterion reworded?"
door: two-way
blast_radius: external
decider: "human:ryan"
provenance: user-approved
decided_at: "2026-10-19T17:49:24Z"
tree_hash: "1bba3cfc51a63ec88a08fac36fabc31e276588f9"
recommended: "A"
flip_condition: "D2 able to enforce early"
---

# Fold into dogfood

## Context and problem statement

Should #6's live-orchestrate exit criterion be met by folding a 3+ ticket /anneal:orchestrate run into the temper dogfood plus the real weigh-in park test, with the criterion reworded?

#6 asks for a live D2 park, which shadow mode cannot produce.

Raised by an agent (sweep) as a call and captured by Weigh-In as `wi-785d28`. Anchor: `docs/decide.md:163`.

## Decision drivers

- cost

## Considered options

### A · Fold into dogfood (chosen)

3+ tickets under /anneal:orchestrate in the temper run; real-binary park test run locally; criterion reworded

- Holds if one live session is enough evidence.
- If wrong: a second live run later.

### B · Separate run

a dedicated orchestrate session

- Holds if fan-out needs its own repo.
- If wrong: about 15x the tokens spent and still no park.

### C · Tests only

close #6 on tests and move the live check to post-v0.1

- Holds if tests are enough evidence.
- If wrong: a live fan-out bug ships.

## Evidence

- `docs/decide.md:163`: stays in shadow (data.configured_mode: enforce) until it has 50 answered decisions
- `crates/anneal-cli/tests/it/orchestrate.rs:311`: fn the_pinned_weigh_in_parks_once_and_its_ruling_reaches_d6

## Decision outcome

Chosen option: **Fold into dogfood**, the recommended option.

Decider's note: Ryan tapped 'Fold into dogfood' on the card, 2026-10-19

### Consequences

- Good, because it holds if one live session is enough evidence.
- Bad, because if wrong: a second live run later.

### Confirmation

Revisit when: D2 able to enforce early

## Why this might be wrong

the temper change may not split into 3 independent tickets

## Provenance

- Decided 2026-10-19T17:49:24Z by human:ryan (user-approved), via cli.
- Raised in session `9150ad3e-ef35-5984-8579-fdd986e951f0`.
- Verbatim: "Should #6's live-orchestrate exit criterion be met by folding a 3+ ticket /anneal:orchestrate run into the temper dogfood plus the real weigh-in park test, with the criterion reworded?"
