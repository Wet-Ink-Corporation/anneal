---
id: "kb-decision-wi-077990"
title: "Should `verdict set` go through the proxy so cloud sessions can post `anneal/verdict` before it becomes a required check: A"
kind: decision
status: accepted
authority_tier: decision
summary: "In the context of \"Should `verdict set` go through the proxy so cloud sessions can post `anneal/verdict` before it becomes a required check?\", facing the question an agent raised, we decided for A."
depends_on: []
related: []
source_paths: []
last_reviewed: "2026-11-01"
reversibility: medium
phase: null
supersedes: []
superseded_by: null
weighin_item: "wi-077990"
question: "Should `verdict set` go through the proxy so cloud sessions can post `anneal/verdict` before it becomes a required check?"
door: unsure
decider: "human:claude"
provenance: user-directed
decided_at: "2026-11-01T00:07:21Z"
tree_hash: "9a13740513948eebce72470d75fbba6dfaafa5a4"
---

# Should `verdict set` go through the proxy so cloud sessions can post `anneal/verdict` before it becomes a required check: A

## Context and problem statement

Should `verdict set` go through the proxy so cloud sessions can post `anneal/verdict` before it becomes a required check?

Raised by an agent (marker) as a call and captured by Weigh-In as `wi-077990`. Anchor: `crates/anneal-cli/src/verdict/commit_status/http.rs:1`.

## Decision outcome

Chosen option: **A**.

Decider's note: Ryan picked A on the card 2026-11-01: the commit-status post honours HTTPS_PROXY and SSL_CERT_FILE; Clef stays direct; no new dependency

## Provenance

- Decided 2026-11-01T00:07:21Z by human:claude (user-directed), via cli.
- Raised in session `28a41a8d-0d5a-561f-949d-181904faa20f`.
- Verbatim: "kind: call"
