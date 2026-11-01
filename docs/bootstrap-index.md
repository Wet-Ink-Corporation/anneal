# anneal

anneal turns SDLC gates into enforced checks. Skills say how to work, the `anneal` CLI and its
hooks check that it was done, and the ledger records what happened.

## Entry skills

Load the matching skill before your first tool call:

- `grill`: new behaviour, even a precise rule, without saying how it works, or a vague request; question it until confirmed.
- `spec`: requirements are agreed; write the spec and its criteria.
- `plan`: a spec is ready; split it into tasks and get the plan approved.
- `build`: code to add or change, or an approved task; build it test-first.
- `diagnose`: told to fix wrong behaviour or a failing test; reproduce it red before any edit.
- `verify`: is it fixed or tested, or work looks done; prove it first.
- `ship`: verified work needs a pull request, verdict, merge or rollout.

## User commands

The human runs these; do not start them unasked:

- `/anneal:orchestrate`: run a plan's tasks across subagents.
- `/anneal:handoff`: write a brief for the next session.
- `/anneal:correct`: record a correction as a standing order.
- `/anneal:write-skill`: author a skill that lints clean.
- `/anneal:calibrate`: calibrate a Clef decision point.
