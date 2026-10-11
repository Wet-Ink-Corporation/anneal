# Slicing: tracer bullets and blocking edges

A plan's tasks are vertical slices. Each one is a thin path through every layer the feature
touches (input, logic, storage, output), and it ends in a test that passes. The first slice is a
tracer bullet: the simplest end-to-end case, built first so every later task extends something
that already runs.

## Cutting slices

| Cut | Do | Avoid |
|---|---|---|
| Direction | Through the layers: one behaviour, end to end | Along them: all models, then all handlers, then all views |
| First task | The happy path's simplest case, end to end | Setup, scaffolding or "core types" with no test of behaviour |
| Size | One acceptance criterion, or part of one, and its test | Several criteria in one task |
| Done | Its `Test:` command passes and nothing else is red | "Compiles", or "ready for the next task" |
| Order | Happy path, then edge cases, then errors | Errors before anything works |

A slice that cannot end green alone is two slices, or half of one. Merge or re-cut it.

## Blocking edges

`Blocked by:` lists only the tasks whose output this task needs: a type it uses, a table it
reads, a command it calls. Sharing a file is not a block; sequencing by habit is not a block.
Fewer edges mean more tasks a team (or `/anneal:orchestrate`) can run at once.

- Every blocker must be another task of the plan; `anneal lint plan` reports one that is not,
  a task blocked by itself, and a cycle.
- A blocker outside the plan (another team's ticket, a release) belongs in the tracker, not
  in `Blocked by:`.
- When the tasks become tickets, `anneal dag check` checks the same graph across the tracker
  (cycles, dangling blockers) and `anneal dag frontier` lists what can start now.

## Signs the slicing is wrong

- The first green test arrives in the third task or later.
- Most tasks are blocked by the one before them: a chain, not a graph.
- A task's `Files:` line spans every layer of the codebase.
