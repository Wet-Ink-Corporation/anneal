# Dry run: the define chain on one small idea

The idea: **add a `--json` flag to the `notes list` command of a toy notes CLI**, so people
scripting their notes can pipe the list into `jq`. It was taken through grill, spec, plan and
tickets with the real `anneal` binary (story #21), and the artifacts are kept here as a
regression fixture.

`crates/anneal-cli/tests/it/dry_run.rs` replays the deterministic part on a temporary copy of
this directory on every `cargo test --workspace`, so CI fails if a change to `anneal` (or to
these files) breaks the chain.

## The chain

Run from this directory, with a scratch ledger (`--store`) so nothing lands in the
repository's ledger:

| Step | Command | Expect |
|---|---|---|
| grill | `anneal grill frontier --tree design-tree.json` | exit 0, `frontier` empty, `open` 0, `restate` true |
| grill | `anneal grill confirm --tree design-tree.json --quote "Yes, that's it: only notes list, compact JSON, go ahead." --session dry-run --store ledger.sqlite3` | exit 0, one `grill_confirmed` |
| spec | `anneal lint spec spec.md` | exit 0, `needs_clarification` 0, constitution version 1 with 3 gates, glossary read |
| plan | `anneal lint plan plan.md` | exit 0, 4 tasks, `spec` is `spec.md` |
| plan | `anneal plan approve plan.md --session dry-run --store ledger.sqlite3` | exit 0, one `plan_approved` |
| plan | `anneal plan check plan.md --store ledger.sqlite3` | exit 0, `status` `current` |
| tickets | `anneal dag check --dir tickets` | exit 0, 4 tickets, 2 edges |
| tickets | `anneal dag frontier --dir tickets` | exit 0, frontier `T1`, `T4` |

## Files

| File | What it is |
|---|---|
| `design-tree.json` | The grill's design tree (`plugin/playbooks/schemas/design-tree.schema.json`): 7 nodes answered, 1 deferred (`pretty`), two turns ending at confidence 0.85. |
| `CONTEXT.md` | The toy project's glossary; `lint spec` reads it by default. |
| `constitution.md` | The toy project's constitution, version 1, gates G1 to G3; `lint spec` reads it by default. |
| `spec.md` | The spec, from `plugin/playbooks/spec.template.md`: five EARS criteria, no open marker. |
| `plan.md` | The plan, from `plugin/playbooks/plan.template.md`, linking `spec.md`: tasks T1 to T4. |
| `tickets/T1.md` .. `T4.md` | The plan's tasks as markdown tickets (`docs/dag.md`), ids and blockers as in the plan. |
| `events.jsonl` | The ledger after the chain, one `ledger query` event per line, with `ts`, `worktree`, `branch` and `head` removed. |

The hashes in `events.jsonl` are the sha256 of `design-tree.json` and `plan.md` as checked in
(LF line endings, from `.gitattributes`). Editing either file means re-running the two ledger
steps above on a fresh `--store` and re-recording the events, for example with
`anneal ledger query --store ledger.sqlite3 | jq -c '.data.events[] | .event |= del(.ts, .worktree, .branch, .head)'`.

## Friction found

What got in the way while running the chain by hand, and the check each suggests (mistakes
become checks):

1. **Nothing turns plan tasks into tickets.** The plan's `### T1` tasks and their
   `Blocked by: T1` lines were copied into `tickets/*.md` frontmatter by hand, and nothing
   checks that the two agree: a ticket could drop a blocker and `dag check` would still pass.
   Candidate check: `anneal dag check --plan plan.md` (or a `plan tickets` export) that fails
   when the tickets' ids and blockers differ from the plan's tasks.
2. **`plan check` does not see a spec change.** `plan_approved` records only the plan's hash;
   after approval, adding a line to `spec.md` still checks `current`, while `--help` reads
   "4 stale: inputs changed since approval" and `proves` says "current with its inputs".
   `docs/plan.md` documents the gap. Candidate check: record the spec's hash in
   `plan_approved` and exit 4 when it changes, or reword the help and `proves` to say plan only.
   #76 took the rewording: the help, `proves` and `docs/exit-codes.md` now say only the plan's
   bytes are compared.
3. **The spec does not name the design tree it came from.** The spec skill says to write only
   from what the grill confirmed, but no field links `spec.md` to `design-tree.json` or its
   `grill_confirmed` event, so `lint spec` cannot tell a confirmed spec from an invented one.
   Candidate check: a `Tree:` header line in the spec that `lint spec` resolves, and a
   `plan approve` refusal when no `grill_confirmed` carries that tree's current hash.
4. **`grill_confirmed` names the tree under `plan`.** The event's path field is `plan`, but it
   holds the design tree's path, which reads wrongly in `ledger query` output. Candidate fix: a
   `tree` field in the next registry version.
5. **The token estimate counts mentioned files.** A ticket body that mentions `` `plan.md` ``
   for context gets that file's size added to its estimate, as if the ticket would edit it.
   Paths are sized from the working directory, so the same tickets estimate differently from
   another directory. Candidate rule for the plan skill: name only files the ticket touches in
   backticks; or a `files:` list that, when present, replaces the body scan.
