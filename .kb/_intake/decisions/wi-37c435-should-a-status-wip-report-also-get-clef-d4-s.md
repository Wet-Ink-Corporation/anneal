---
id: "kb-decision-wi-37c435"
title: "WIP like BLOCKED"
kind: decision
status: accepted
authority_tier: decision
summary: "In the context of \"Should a Status: WIP report also get Clef D4's section check, as BLOCKED does, so a bare WIP pause can be blocked in effect enforce?\", facing accepting WIP made it a status block, so Clef D4 now checks its sections too, we decided for WIP like BLOCKED and neglected Exempt WIP from D4, on the premise that pauses should still report gate and review, accepting that if wrong: a bare WIP pause can be blocked in enforce."
depends_on: []
related: []
source_paths: []
last_reviewed: "2026-11-09"
reversibility: high
phase: null
supersedes: []
superseded_by: null
weighin_item: "wi-37c435"
question: "Should a Status: WIP report also get Clef D4's section check, as BLOCKED does, so a bare WIP pause can be blocked in effect enforce?"
door: two-way
blast_radius: module
decider: "human:claude"
provenance: user-approved
decided_at: "2026-11-09T00:30:31Z"
tree_hash: "bdbdfcc43748de53098aa8e7cd83224cf7d36b06"
recommended: "A"
flip_condition: "bare WIP pauses get blocked in practice"
---

# WIP like BLOCKED

## Context and problem statement

Should a Status: WIP report also get Clef D4's section check, as BLOCKED does, so a bare WIP pause can be blocked in effect enforce?

Accepting WIP made it a status block, so Clef D4 now checks its sections too.

Raised by an agent (sweep) as a call and captured by Weigh-In as `wi-37c435`. Anchor: `crates/anneal-cli/src/hook/status/block.rs:95`.

## Decision drivers

- consistency

## Considered options

### A · WIP like BLOCKED (chosen)

D4 checks WIP reports' sections

- Holds if pauses should still report gate and review.
- If wrong: a bare WIP pause can be blocked in enforce.

### B · Exempt WIP from D4

record WIP, skip D4 for it

- Holds if a pause should never be blocked.
- If wrong: ungrounded WIP reports pass unchecked.

## Evidence

- `crates/anneal-cli/src/hook/status/block.rs:95`: section_texts reads STATUSES

## Decision outcome

Chosen option: **WIP like BLOCKED**, the recommended option.

Decider's note: Ryan tapped A on the card

### Consequences

- Good, because it holds if pauses should still report gate and review.
- Bad, because if wrong: a bare WIP pause can be blocked in enforce.

### Confirmation

Revisit when: bare WIP pauses get blocked in practice

## Why this might be wrong

temper's own hook accepts a bare WIP

## Provenance

- Decided 2026-11-09T00:30:31Z by human:claude (user-approved), via chat.
- Raised in session `ce701b16-9785-59d6-aa30-13e1f5f1c96d`.
- Verbatim: "Should a Status: WIP report also get Clef D4's section check, as BLOCKED does, so a bare WIP pause can be blocked in effect enforce?"
