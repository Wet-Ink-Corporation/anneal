# The ledger

The ledger records what happened in a repository's sessions: tests run, tasks done, plans
approved, gates and stops blocked. Hooks and commands append to it; gates query it. It is a
[happenstance](https://crates.io/crates/happenstance-sqlite) SQLite event store (0.3.2, the
version weigh-in runs), kept by the `anneal-ledger` crate and driven by `anneal ledger`.

## Where it lives

One file per repository, shared by every worktree and never committed:
`<git-common-dir>/anneal/ledger.sqlite3`. A command resolves the store in this order:

1. `--store <file>` (relative paths are taken from the working directory);
2. `$ANNEAL_STORE` (empty counts as unset);
3. the anneal config file (there is none yet; the step is reserved);
4. `<git-common-dir>/anneal/ledger.sqlite3`, the same file from the main checkout and every
   linked worktree;
5. `<folder>/.anneal/ledger.sqlite3` outside git: only when git says the directory is not in a
   repository. Any other git failure (for example "detected dubious ownership", or a broken
   `.git` file) stops the command with exit 70, so a repository git refuses never gets a
   second, per-folder ledger.

Query results report the file in `data.store` and the step that chose it in `data.source`
(`flag`, `env`, `git_common_dir` or `folder`).

## Storage rules

- **TRUNCATE journal, never WAL.** WAL creates `-wal` and `-shm` files and deletes them on
  close; Cowork's connected folders refuse deletes and WAL's shared memory. The ledger keeps one
  `ledger.sqlite3-journal` beside it, emptied at each commit and never deleted. It is never
  opened through happenstance's own `SqliteEventStore::open`, which switches to WAL: anneal opens
  its own connection and copies happenstance's schema in from a template, inside one
  `BEGIN IMMEDIATE`. A file already in WAL is refused, not converted.
- **Refused files are left as they were.** Opening only reads until the file is known to be a
  ledger this anneal reads, or an empty database. A ledger stamped by a newer anneal, a file in
  WAL, and a SQLite database with tables that is not a ledger (say, a wrong `--store` or
  `$ANNEAL_STORE`) are each refused with exit 70, and nothing in them changes. The schema
  version and WAL are read from the file's 100-byte header before SQLite opens it, so refusing
  a WAL file creates no `-wal` or `-shm` beside it. Only a new file or an empty database is
  given the ledger schema.
- **Schema version.** `PRAGMA user_version` holds the ledger schema (1: happenstance 0.3.2's
  schema under TRUNCATE), stamped in the transaction that creates the schema.
- **Concurrent writers.** Every append is its own `BEGIN IMMEDIATE` transaction, and each
  connection waits up to 10 seconds for another's lock, so concurrent hooks serialise.
- **Conditioned appends (DCB).** `Store::append_if` writes only if no event matching a condition
  exists (optionally: after a position the writer has read). It is checked inside the append's
  transaction, so of two writers racing on the same condition exactly one wins; the other gets
  `Conflict`. Gates that need first-writer-wins use it: `anneal plan approve` appends
  `plan_approved` only if no other approval of the plan landed after the one it read.

## The envelope

Every event is one JSON object: the event's `type` and payload, flattened beside its context.

| Key | Type | Meaning |
|---|---|---|
| `type` | string | The event type, snake case, from the registry below |
| *payload* | | The type's fields (below) |
| `actor` | string | Who wrote it: `cli`, `hook`, `agent` or `human:<name>` |
| `session_id` | string or null | The writing agent session |
| `worktree` | string or null | The writer's git worktree; `anneal mode` writes its top-level directory as `git rev-parse --show-toplevel` prints it |
| `branch` | string or null | The checked-out branch |
| `head` | string or null | The commit at `HEAD` |
| `ts` | string | When it was written, RFC 3339 (anneal writes UTC seconds: `2026-10-11T09:30:00Z`) |

```json
{"type":"task_done","task":"T1","actor":"cli","session_id":"s1","worktree":null,"branch":null,"head":null,"ts":"2026-10-11T09:30:00Z"}
```

Every payload field name is a persisted JSON key: renaming one is a migration. Optional fields
read back from either `null` or a missing key, so adding one is additive. Writing is strict: an
unregistered `type`, a missing or mistyped field, a key the type does not define, a `ts` that is
not RFC 3339 or a tag value that is not valid is a schema error (exit 2) and nothing is written.
Reading ignores keys a newer anneal added.

## Tags

The store indexes events by `key:value` tags. The keys are a closed vocabulary:

| Tag | Carried by |
|---|---|
| `session:<session_id>` | every event with a `session_id` |
| `worktree:<worktree>` | every event with a `worktree` (`mode_set` is selected by it) |
| `task:<task>` | `test_run` (when it names a task), `task_done`, `repro_red` |
| `plan:<plan>` | `plan_approved`, `grill_confirmed` |
| `pr:<number>` | `verdict_set`, `round_started` |
| `point:<point>` | `gate_blocked`, `stop_blocked` |

A tag value must be non-empty, free of control characters and at most 255 bytes with its key.

## Event registry (version 4)

Each type's minimal payload. Later stories extend payloads with optional fields (bumping the
registry version) or add types. Version 2 added `hash` and `quote` to `grill_confirmed` (#16);
version 3 added `exit`, `duration_ms` and `timeout_secs` to `test_run` (#22, `docs/task.md`);
version 4 added `mode_set` (#23, `docs/preconditions.md`).

| Type | Payload | Subject tag |
|---|---|---|
| `test_run` | `task?` string, `command` string, `outcome` `green`/`red`, `exit?` integer (`null` when killed), `duration_ms?` integer, `timeout_secs?` integer (set only when a timeout killed the command) | `task:` |
| `task_done` | `task` string | `task:` |
| `plan_approved` | `plan` string (the plan's path as given to `anneal plan approve`, `.` components dropped; normally from the repository root), `hash` string (`sha256:<64 hex digits>` of the approved bytes; `docs/plan.md`) | `plan:` |
| `grill_confirmed` | `plan` string (for `anneal grill confirm`, the design tree's path as given), `hash?` string (`sha256:` and 64 hex digits of the confirmed tree), `quote?` string (the human's confirmation, verbatim); see [grill.md](grill.md) | `plan:` |
| `repro_red` | `task` string, `test` string (the failing reproduction) | `task:` |
| `verdict_set` | `pr` number, `verdict` `approved`/`changes_requested`/`noise_only` | `pr:` |
| `round_started` | `pr` number, `round` number (from 1) | `pr:` |
| `gate_blocked` | `point` string (the gate), `reason` string | `point:` |
| `stop_blocked` | `point` string (the check that blocked the stop), `reason` string | `point:` |
| `mode_set` | `mode` `build`/`diagnose`/`off`, `clean` boolean (no change, untracked files included, at entry), `plan?` string and `tree?` string (build: paths relative to the worktree root, `/`-separated); the latest one for a worktree is its mode, see [preconditions.md](preconditions.md) | `worktree:` (from the envelope) |

## Commands

```text
anneal ledger [--store <file>] append '<event json>' [--actor <who>] [--session <id>]
anneal ledger [--store <file>] query [--type <type>]... [--tag <key:value>]... [--session <id>]
anneal ledger [--store <file>] tail [-n <N>] [same filters as query]
anneal ledger [--store <file>] journal
```

- `append` fills `actor` (default `cli`), `session_id` (from `--session`) and `ts` (now) when
  the event leaves them out or sets them to `null`. Its envelope's `data` holds `store`,
  `source`, `position` and the stored `event`.
- `query` and `tail` filter by any of the `--type`s and all of the `--tag`s (`--session <id>` is
  `--tag session:<id>`). Their `data` holds `store`, `source`, `registry` and `events`, each
  `{"position": n, "event": {...}}` in position order. `tail` keeps the last `N` (default 10).
- `journal` prints every event's envelope as one JSON line, in position order, with no result
  envelope: the output is a journal file a cloud session commits, since its ledger leaves with
  its container.

Exit codes: 0 ok, 2 schema error (an envelope with one `ledger/schema` finding on stdout), and
the global codes in [exit-codes.md](exit-codes.md) (70 when the store cannot be opened or
written).
