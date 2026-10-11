---
id: "kb-decision-wi-08319f"
title: "TypeScript + Node, tsc and claude CLI in CI"
kind: decision
status: accepted
authority_tier: decision
summary: "In the context of \"May the repo add TypeScript (the cockpit mod) and install Node plus the Claude Code CLI in CI for tsc, plugin validate and plugin test?\", facing a mod must be JS or TS, and validating it needs the claude CLI in CI, we decided for TypeScript + Node, tsc and claude CLI in CI and neglected Plain JS, validate only; No CI for the mod, on the premise that we want the mod tested like the CLI, accepting that if wrong: one more CI job to drop."
depends_on: []
related: []
source_paths: []
last_reviewed: "2026-10-11"
reversibility: high
phase: null
supersedes: []
superseded_by: null
weighin_item: "wi-08319f"
question: "May the repo add TypeScript (the cockpit mod) and install Node plus the Claude Code CLI in CI for tsc, plugin validate and plugin test?"
door: two-way
blast_radius: module
decider: "human:ryan"
provenance: user-approved
decided_at: "2026-10-11T15:40:50Z"
tree_hash: "955c5d1766dc0040a710a71bbc7e062086329e95"
recommended: "A"
flip_condition: "Ryan wants the mod kept experimental"
---

# TypeScript + Node, tsc and claude CLI in CI

## Context and problem statement

May the repo add TypeScript (the cockpit mod) and install Node plus the Claude Code CLI in CI for tsc, plugin validate and plugin test?

A mod must be JS or TS, and validating it needs the claude CLI in CI.

Raised by an agent (sweep) as a call and captured by Weigh-In as `wi-08319f`. Anchor: `https://github.com/Wet-Ink-Corporation/anneal/issues/57`.

## Decision drivers

- Catches API drift between Claude Code releases
- Added CI time

## Considered options

### A · TypeScript + Node, tsc and claude CLI in CI (chosen)

Strict tsc, plugin validate --strict and plugin test on 3 OSes

- Holds if we want the mod tested like the CLI.
- If wrong: one more CI job to drop.

### B · Plain JS, validate only

No tsc; claude plugin validate in CI, no plugin test

- Holds if the mod stays tiny.
- If wrong: UI regressions found by hand.

### C · No CI for the mod

Manual checks only

- Holds if the mod is experimental.
- If wrong: breakage ships silently when the mods API moves.

## Evidence

- `https://code.claude.com/docs/en/plugins/mods/reference`: The hooks module ... Named .js, .mjs, .cjs, .jsx, .ts, .mts, .cts, or .tsx
- `AGENTS.md`: H-16 new dependencies ... are human decisions

## Decision outcome

Chosen option: **TypeScript + Node, tsc and claude CLI in CI**, the recommended option.

Decider's note: Chosen on the decision card: TS plus CI tests

### Consequences

- Good, because it holds if we want the mod tested like the CLI.
- Bad, because if wrong: one more CI job to drop.

### Confirmation

Revisit when: Ryan wants the mod kept experimental

## Why this might be wrong

If the mod stays under 200 lines, B may be enough

## Provenance

- Decided 2026-10-11T15:40:50Z by human:ryan (user-approved), via cli.
- Raised in session `79d77b96-1e25-5b54-bd19-6d40243f7bdd`.
- Verbatim: "May the repo add TypeScript (the cockpit mod) and install Node plus the Claude Code CLI in CI for tsc, plugin validate and plugin test?"
