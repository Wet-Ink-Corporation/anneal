# Plans: the template, `anneal lint plan` and `anneal plan approve|check`

A plan splits a spec with no open questions into tasks a fresh session can build test-first.
It is written from `plugin/playbooks/plan.template.md` (the `plan` skill says how), checked
by `anneal lint plan`, and approved by the human with `anneal plan approve`, which records the
plan's hash in the ledger. `anneal plan check` then tells whether the plan's bytes are still
the ones that were approved: any edit, even one byte, voids the approval. Only the plan's
bytes are compared; its spec is not (see below).

```text
anneal lint plan <FILE>
anneal plan approve <FILE> [--actor human|model] [--via mod|bash|terminal] [--session <id>]
                    [--store <ledger>]
anneal plan check <FILE> [--store <ledger>]
```

## Format

A plan is markdown. Fenced code blocks and HTML comments are not read, as for specs
(`docs/spec.md`).

- **Header**: the lines before the first `##` section hold one `Spec:` line naming the spec:
  a markdown link (`- Spec: [export](../specs/export.md)`, or `[export](<../specs/export.md>)`
  with the destination in angle brackets, read as what lies inside them), a path in backticks
  or a plain path.
  A relative path is read relative to the plan's directory, as a markdown link is; an
  absolute one as written.
- **Sections**: `## Goal`, `## Tasks` and `## Review focus`, each once (titles in any letter
  case); Goal and Review focus hold text. Other sections, such as `## Architecture`, are free.
- **Tasks**: each level-3 heading under `## Tasks` is a task, `### <id>: <title>` or
  `### Task <id>: <title>`. An id is one word of ASCII letters, digits, `-`, `_` or `.`, at
  most 64 bytes, unique in the plan. A task runs to the next level-3 heading; `####` headings
  inside it are its own.
- **Task fields**: one line each, optionally a list item, the key optionally bold
  (`**Files:**` or `**Files**:`), in any letter case:
  - `Files:` the files the task creates or changes, each in backticks;
  - `Test:` the command that runs the task's tests, in backticks (the first span counts);
  - `Blocked by:` `none`, or the ids of the tasks whose output it needs, comma-separated,
    optionally in backticks.

The plan as read is `data.plan` in `lint plan`'s envelope, described by
`plugin/playbooks/schemas/plan.schema.json` (a test keeps the two in agreement):

```json
{"spec": "../specs/export.md",
 "tasks": [{"id": "T1", "title": "CSV writer", "line": 11, "files": ["src/csv.rs"],
            "test": "cargo test -p report csv", "blocked_by": []}]}
```

## `anneal lint plan`

Every finding is an error with a fix string; the run exits 0 when there is none and 1 when
there is any (`lint`'s codes, `docs/exit-codes.md`). In the style of pstack's `check-plan`:
structure only, never the plan's merit.

| Rule | What is checked | Where the finding points |
|---|---|---|
| `spec` | One `Spec:` line in the header names a file, and that file exists | the `Spec:` line; line 0 when there is none |
| `sections` | Goal, Tasks and Review focus present once each; Goal and Review focus not empty | line 0 for a missing one; the heading otherwise |
| `tasks` | Tasks has at least one task; every level-3 heading is `<id>: <title>`; ids are unique | the heading |
| `task-fields` | Each task has one `Files:` line with a file in backticks, one `Test:` line with a command in backticks, and one `Blocked by:` line that is `none` or ids | the field's line; the task's heading for a missing field |
| `blockers` | Every blocker is another task of the plan, and no cycle | the `Blocked by:` line; the first task left unordered for a cycle |

- A missing plan, a directory or a file over 1 MiB is a usage error (exit 2, one `ERROR`
  line, nothing on stdout). Text that is not UTF-8 is read lossily.
- `data` holds `file`, `spec` (the resolved spec path, or `null`), `tasks` (how many were
  read), `plan` and `findings`. `data.plan.tasks` lists at most the first 1,000 tasks in file
  order; `data.omitted_tasks` counts the rest and is present only when some were left out, so
  a 1 MiB plan of unique tasks still gives a small envelope. Each finding has `key`
  (`lint/plan/<rule>:<file>:<line>[:<subject>]`, stable across runs), `rule`, `file`, `line`,
  `message`, `fix` and, where one line can hold several findings, `subject` (a section, a
  field or a blocker id).
- At most 1,000 findings are listed, the first in line order; `data.omitted` counts the rest
  and is present only when some were dropped. A plan within the 1 MiB cap can hold about
  100,000 task headings, and a finding for each would make an envelope of about 100 MB. The
  run still exits 1, and `plan approve` lists the same findings and `data.omitted`.
- A blocker outside the plan belongs in the tracker: `anneal dag check` checks the ticket
  graph (see the `plan` skill's `references/slice.md`).

## `anneal plan approve`

Run only on the human's explicit approval of the plan they were shown. It refuses, with exit
3 (`not approved`), a finding, and nothing appended (no ledger is opened):

- while `lint plan` has any finding (the findings are listed);
- while the linked spec has any `[NEEDS CLARIFICATION]` marker, counted as `lint spec` counts
  them (outside code, comments and fences); `data.needs_clarification` gives the count.

Then, when the spec's header has a `Tree:` line (`docs/spec.md`), it refuses the same way
(exit 3, nothing appended) unless one of the latest 256 `grill_confirmed` events carries the
`sha256:` hash of that design tree's current bytes, matched by hash as the preconditions hook
matches it (`docs/preconditions.md`), so any path spelling of the tree counts (#77):

- a `Tree:` line naming no file, still the template's placeholder, or naming a file that does
  not exist or is not a regular file: a `plan/approve/tree:<spec>` finding;
- a tree with no such confirmation, never confirmed or edited since: a
  `plan/approve/unconfirmed-tree:<tree>` finding naming `anneal grill confirm`, and
  `data.tree` names the tree. A ledger that does not exist holds no confirmation and is not
  created; a tree that exists but cannot be read, or a ledger that cannot be read, fails with
  exit 70.

A spec with no `Tree:` line is not checked, so specs written before #77 still approve.
`data.tree` is present only when the spec names a tree: the resolved path, in a refusal about
it and in an approval.

Otherwise it appends `plan_approved {plan, hash}` to the ledger (`docs/ledger.md`) and exits
0. `plan` is the plan id: the path as given, separated by `/` on every platform, with `.` and empty
components dropped, so `./docs/plans/x.md` and `docs/plans/x.md` are one plan (`..` is
kept, leading separators are kept as written, so an absolute path stays absolute and a UNC
path keeps its `//`, and the path is not made absolute: approve and check a plan from the
same directory, normally the repository root). `hash` is `sha256:` and
the 64 lowercase hex digits of the SHA-256 of the plan's bytes. When the latest approval of the
plan already has that hash, nothing is appended and `data.appended` is `false`.

`--actor` (who approved: `human` or `model`) and `--via` (through what: `mod`, the cockpit's
buttons, `bash`, a model's Bash call, or `terminal`) are recorded as the approval's `by`
(`{"actor": "model", "via": "bash"}`) and shown in `data.by`; each is optional and none is
assumed, so an approval without them records no approver, as before (#62). The model may
approve a plan (Ryan, wi-720468: B), so the guard allows `plan approve` with any actor. An
approval counts as already recorded only when it also names the same approver, so a human's
approval of content a model approved is appended and is the latest. `plan check` still
compares only the hash. The append is conditioned on no other approval of the plan landing
since the one it read. When one did, it is read back: with the same hash and approver the plan
is already approved (exit 0, `data.appended` `false`); with the same hash but another approver
(or none) this approval is still news and is appended after it, under the same condition, up
to 8 times in a row; with another hash the approval is refused (exit 3, a
`plan/approve/conflict:<plan>` finding: run `plan check`, then approve again if it is not
current). Losing the race to the same hash 8 times in a row is refused the same way.

## `anneal plan check`

Compares the hash of the plan's bytes, and nothing else, with the latest `plan_approved` for
its id:

| Code | Meaning | `data.status` |
|---|---|---|
| 0 | approved and current | `current` |
| 3 | not approved: no approval of this plan id | `not_approved` |
| 4 | stale: the plan's bytes changed since its latest approval | `stale` |

`data` also holds `plan`, `hash`, `approved` (`{hash, position, ts}` of the latest approval,
or `null`), `store` and `source`. `check` never creates a ledger: when the ledger path does
not exist, the plan is not approved; any other ledger it cannot open (a directory, an
unreadable file) fails with exit 70 rather than reading as not approved.

`check` compares only the plan's bytes. `plan_approved` records no hash of the spec, so a
spec edited after approval still checks `current` (exit 0); such an edit is the reviewer's to
catch. The `--help` text, the green-result claim and `docs/exit-codes.md` say the same (#76).
Recording the spec's hash would need a ledger registry version bump, a decision left open.
