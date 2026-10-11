# anneal

anneal turns SDLC gates into enforced checks. Skills say how to work, the `anneal` CLI and its
hooks check that it was done, and the ledger records what happened.

## Entry skills

Load the skill whose trigger matches before acting:

- `grill`: a request is new or vague; question it until the requirements are confirmed.
- `spec`: requirements are agreed; write the spec and its acceptance criteria.
- `plan`: a spec is ready; split it into tasks with a dependency graph and get the plan approved.
- `build`: an approved task is next; implement it test-first.
- `diagnose`: something is broken or a test fails unexpectedly; reproduce it red before fixing.
- `verify`: work looks done; check it against the spec before claiming it.
- `ship`: verified work needs a pull request, review verdict, merge or rollout.

## User commands

The human runs these; do not start them unasked:

- `/anneal:orchestrate`: run a plan's tasks across subagents.
- `/anneal:handoff`: write a brief for the next session.
- `/anneal:correct`: record a correction as a standing order.
- `/anneal:write-skill`: author a skill that passes `anneal lint skills`.
- `/anneal:calibrate`: calibrate a Clef decision point.
