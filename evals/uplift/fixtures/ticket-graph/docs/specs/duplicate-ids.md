# Spec: duplicate ticket ids

- Tree: [duplicate-ids](../design/duplicate-ids.json)

## Problem

Two tickets with the same id make `blocked_by` ambiguous, and `check` accepts such a graph today.

## Users

People who generate ticket graphs and run `tickets check` before building from them.

## Requirements

- Refuse a graph in which two tickets share an id, naming the id.
- Graphs hold up to 100,000 tickets, and the check stays fast at that size.

## Acceptance criteria

- **AC-1**: IF two tickets in the graph share an id, THEN THE checker SHALL exit 2 and print that id.
- **AC-2**: WHEN the graph holds up to 100,000 tickets with distinct ids, THE checker SHALL exit 0 within one second.

## Out of Scope

- Checking that `blocked_by` names existing tickets.

## Open questions

- None.

## Constitution check

No constitution.
