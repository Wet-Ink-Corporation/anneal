# Red, green, refactor

One behaviour per cycle, in the order the task's criteria give.

| Step | Do | Done when |
|---|---|---|
| Red | Write one test for the next behaviour; run the task's `Test:` command | It fails, and the message names the missing behaviour (a compile error about the missing item counts) |
| Green | Write the least code that passes; no shortcut you would not merge | The command passes, and every other test still does |
| Refactor | Fix names, duplication and placement with the tests green | The command still passes, and no test changed |

## Rules

- A test that never failed has proved nothing. Watch it fail first, for the right reason.
- A bug starts with a test that reproduces it red; if the cause is unknown, that is `diagnose`.
- Editing a test during refactor means the behaviour changed: go back to red.
- Never weaken a test to get green: deleting, skipping or loosening one is a lowered floor, and
  `anneal floor-guard` reports it.

## Completion goes through `anneal task done`

A task is done when its test command passes under anneal, not when the code looks finished:

```sh
anneal task done T3 -- cargo test -p notes list_is_sorted
```

It runs the command without a shell, records a `test_run` with its exit code and duration, and
records `task_done` only on exit 0. A red or timed-out run records the red `test_run` alone and
exits 1, so the task stays open. In a Rust repository, run `temper:gate` before it, at every task
boundary.
