---
id: "kb-decision-wi-1fa610"
title: "May the #157 test renames and moves, the stale comment in an existing test, and the calibrate label-map cleanup go ahead: A"
kind: decision
status: accepted
authority_tier: decision
summary: "In the context of \"May the #157 test renames and moves, the stale comment in an existing test, and the calibrate label-map cleanup go ahead?\", facing the question an agent raised, we decided for A."
depends_on: []
related: []
source_paths: []
last_reviewed: "2026-11-01"
reversibility: high
phase: null
supersedes: []
superseded_by: null
weighin_item: "wi-1fa610"
question: "May the #157 test renames and moves, the stale comment in an existing test, and the calibrate label-map cleanup go ahead?"
door: two-way
decider: "human:claude"
provenance: user-directed
decided_at: "2026-11-01T00:11:45Z"
tree_hash: "149861895f728821162ade0aba016e418bf4dcce"
---

# May the #157 test renames and moves, the stale comment in an existing test, and the calibrate label-map cleanup go ahead: A

## Context and problem statement

May the #157 test renames and moves, the stale comment in an existing test, and the calibrate label-map cleanup go ahead?

Raised by an agent (marker) as a deferral and captured by Weigh-In as `wi-1fa610`. Anchor: `crates/anneal-cli/src/hook/guard/denylist/tests/human_only.rs:361`.

## Decision outcome

Chosen option: **A**.

Decider's note: Ryan picked A on the card 2026-11-01: rename the writer test, move spec_path/slashed and verdict-name tests, fix the human_only comment, single calibrate Label map; no expectation changes; #62 F1 stays with #128

## Provenance

- Decided 2026-11-01T00:11:45Z by human:claude (user-directed), via cli.
- Raised in session `28a41a8d-0d5a-561f-949d-181904faa20f`.
- Verbatim: "kind: deferral"
