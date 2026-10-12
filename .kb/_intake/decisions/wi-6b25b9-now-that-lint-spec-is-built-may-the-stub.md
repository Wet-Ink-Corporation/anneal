---
id: "kb-decision-wi-6b25b9"
title: "Now that `lint spec` is built, may the stub-command test `a_stub_exits_64_with_one_error_line_and_no_stdout` take `lint plan -q` as its stub example instead of `lint spec -q`, with every assertion unchanged (and later `lint brief -q` when #20 builds `lint plan`): accept"
kind: decision
status: accepted
authority_tier: decision
summary: "In the context of \"Now that `lint spec` is built, may the stub-command test `a_stub_exits_64_with_one_error_line_and_no_stdout` take `lint plan -q` as its stub example instead of `lint spec -q`, with every assertion unchanged (and later `lint brief -q` when #20 builds `lint plan`)?\", facing the question an agent raised, we decided for accept."
depends_on: []
related: []
source_paths: []
last_reviewed: "2026-10-12"
reversibility: high
phase: null
supersedes: []
superseded_by: null
weighin_item: "wi-6b25b9"
question: "Now that `lint spec` is built, may the stub-command test `a_stub_exits_64_with_one_error_line_and_no_stdout` take `lint plan -q` as its stub example instead of `lint spec -q`, with every assertion unchanged (and later `lint brief -q` when #20 builds `lint plan`)?"
door: two-way
decider: "human:ryan"
provenance: user-directed
decided_at: "2026-10-12T01:10:57Z"
tree_hash: "1ec44260da3b6a1df87ac177eec64a888c6c2fcc"
---

# Now that `lint spec` is built, may the stub-command test `a_stub_exits_64_with_one_error_line_and_no_stdout` take `lint plan -q` as its stub example instead of `lint spec -q`, with every assertion unchanged (and later `lint brief -q` when #20 builds `lint plan`): accept

## Context and problem statement

Now that `lint spec` is built, may the stub-command test `a_stub_exits_64_with_one_error_line_and_no_stdout` take `lint plan -q` as its stub example instead of `lint spec -q`, with every assertion unchanged (and later `lint brief -q` when #20 builds `lint plan`)?

Raised by an agent (sweep) as a call and captured by Weigh-In as `wi-6b25b9`. Anchor: `crates/anneal-cli/src/cli/tests.rs`.

## Decision outcome

Chosen option: **accept**.

Decider's note: Ryan: "ok" in the project thread, 2026-10-12, accepting the test changes as made

## Provenance

- Decided 2026-10-12T01:10:57Z by human:ryan (user-directed), via chat.
- Raised in session `fd5b0e17-6cb7-568a-8c93-ccf32f4e1e71`.
- Verbatim: "Now that `lint spec` is built, may the stub-command test `a_stub_exits_64_with_one_error_line_and_no_stdout` take `lint plan -q` as its stub example instead of `lint spec -q`, with every assertion unchanged (and later `lint brief -q` when #20 builds `lint plan`)?"
