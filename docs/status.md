# `anneal status`: one read model for every UI

`anneal status` folds the ledger and the anneal home's state files into one document: the
state a person needs to see. A UI (the cockpit mod, a status pane, a script) renders it and
holds no logic of its own.

```text
anneal status [--json] [--fields <section>,...] [--tail <n>]
```

- Without `--json` it prints the text form: one line per section present, then one line for
  any problems, at most 12 lines of at most 200 characters. A control character (line breaks
  and escape sequences among them), U+2028, U+2029 and any other format or default-ignorable
  character is shown as a space, so recorded text cannot break a line or drive the terminal.
- With `--json` it prints the usual envelope (`docs/exit-codes.md`), whose `data` is the
  document below.
- `--fields phase,plan,gates` limits the work to the sections named (any of `phase`, `plan`,
  `tasks`, `gates`, `verdict`, `clef`, `context`, `ledger_tail`); an unnamed section reads
  nothing, so a frequent refresh can ask only for what it shows. An unknown name is a usage
  error.
- `--tail <n>` is how many events `ledger_tail` lists: 10 by default, 0 to 1000.

Exit 0 when a ledger exists; 2 when none does, still with the document (only `clef` and
`context` can have data then) and a `status/no-ledger` finding. It never creates a ledger. The
ledger is the one `anneal ledger` resolves (`$ANNEAL_STORE`, else the git common dir, else
`.anneal/` outside git); the worktree is the one git finds from the working directory.

## The schema

The document carries `"schema": 1`. **Fields are only ever added within `schema: 1`**: a
field, once shipped, keeps its name, type and meaning, and a reader must ignore fields it does
not know. Removing or changing a field is a new schema number.

Every section is optional. A section is absent when its data source does not exist (no mode
recorded, no verdict, no context reading). When its source exists but cannot be read (a state
file that does not parse, git failing), the section is absent too and the envelope has one
finding `status/unreadable:<section>` saying why; the exit code does not change. A failure to
find the worktree is `status/unreadable:worktree`, and of the anneal home
`status/unreadable:home`. The mode is `phase`'s source, so a mode that cannot be read is
`status/unreadable:phase` even when only `plan` or `tasks` was asked for. `context` has two
sources: when one of them cannot be read, the other's reading is still shown, beside the
`status/unreadable:context` finding.

Positions (`event`) are ledger positions, as `anneal ledger query` prints them; times (`ts`)
are the events' RFC 3339 timestamps.

```json
{
  "schema": 1,
  "phase": {
    "mode": "build",
    "playbook": {"name": "build", "requires": ["test_run"]},
    "event": 1,
    "plan": "docs/plan.md",
    "tree": "docs/tree.json",
    "clean": true
  },
  "plan": {
    "id": "docs/plan.md",
    "status": "approved",
    "hash": "sha256:…",
    "approved": {"hash": "sha256:…", "event": 2, "ts": "2026-10-19T10:00:02Z"}
  },
  "tasks": {"done": 1, "total": 2, "next": {"id": "T2", "title": "CLI flag"}},
  "gates": [
    {"id": "preconditions.plan", "kind": "gate_blocked", "reason": "…", "event": 3,
     "ts": "2026-10-19T10:00:03Z"}
  ],
  "verdict": {
    "pr": 7, "head": "<commit>", "base": "main", "outcome": "valid", "valid": true,
    "verdict": {"event": 4, "verdict": "tests_verified", "source": "local", "sha": "<commit>",
                "fresh": "by_sha"},
    "stale": 0
  },
  "clef": {
    "configured": true,
    "points": [{"point": "D1", "answered": 3, "configured_mode": "enforce", "mode": "shadow"}]
  },
  "context": {"used_tokens": 120000.0, "window": 200000, "window_source": "model",
              "percent": 60.0, "level": "warn"},
  "ledger_tail": [
    {"event": 1, "type": "mode_set", "summary": "mode_set build", "actor": "cli",
     "ts": "2026-10-19T10:00:01Z"}
  ]
}
```

(`clef.points` always lists D1 to D7; one is shown.)

### `phase`

The worktree's anneal mode, from its latest `mode_set` (`mode::active`, as the preconditions
hook reads it); absent with no mode, `off`, or outside a worktree. No ledger event records a
"phase" or "playbook" of its own, so this is the most grounded reading: the mode is the phase,
and its playbook is the Stop hook's rule table for that mode
(`hook::completeness::playbook::RULES`), `requires` naming the events its rules need since the
mode was entered (`test_run` for build, `repro_red` for diagnose). `plan`, `tree` and `clean`
are build mode's; they are absent for diagnose mode and for a build mode recorded without a
plan or tree. Clef D3's routed phase is not shown: it is a per-prompt guess, not state.

### `plan`

Build mode's plan, judged as `anneal plan check` judges it (`plan::approval_record`):
`status` is `approved` (the latest approval has the hash of the plan's current bytes),
`stale` (it has another hash: the plan changed since) or `none` (never approved); `hash` is
the current bytes' `sha256:` hash and `approved` the latest approval, when there is one.
Absent outside build mode; a plan file that cannot be read is a finding.

### `tasks`

Build mode's plan's tasks (read as `lint plan` reads them) done and in all: a task is done
when the worktree has a `task_done` for it since build mode was entered, as the handoff note
counts them (`hook::handoff::facts`). `next` is the first task not done, its title cut to 200
characters; absent when all are done.

### `gates`

The newest 10 `gate_blocked` (a guard, the preconditions, brief or merge-gate hook denied a
call) and `stop_blocked` (the Stop hook refused a stop) events of the ledger, oldest first:
`id` is the gate as the event names it, with its `reason`, `event` and `ts`. These are the
gates that recently blocked. The Stop hook's *open* gates (a success claim without a fresh
green run, a `[DEBUG-<id>]` leftover) are not run here: judging them needs the session's
transcript and a scan of the working tree, too slow for a frequent refresh; the handoff note
lists them. The events carry no worktree, so the list is the ledger's, not one worktree's.
`[]` when nothing ever blocked.

### `verdict`

The review verdict for `HEAD`, judged as `anneal verdict valid` judges it
(`verdict::pull_request::read` and `verdict::assess`). Verdicts are recorded per pull request
and nothing maps `HEAD` to one, so the pull request is the one of the newest `verdict_set`
whose commit is `HEAD`, else of the newest `verdict_set` in the ledger (of its latest 64).
`outcome` is `valid`, `noise_only`, `no_verdict`, `stale` or `changes_requested`; `valid` is
true exactly when it is `valid`. `verdict` is the latest verdict still fresh for `HEAD`, and
its `fresh` says why: `by_sha` (set for this commit) or `by_patch_id` (set for an earlier
commit whose `git patch-id --verbatim` from the same base is `HEAD`'s, so a rebase kept it);
absent when none is fresh. Absent with no verdict in the ledger or an unborn `HEAD`. Each git
call has 2 s.

### `clef`

For each decision point D1 to D7: `answered`, the answered decisions logged for it (the
`decisions/counts.json` the shadow period counts, so rotation of the decision log does not
reset it); `configured_mode` from `clef_mode`; and `mode`, the one in effect: `off` without an
account, `shadow` while a configured `enforce` has fewer than 50 answered decisions, else the
configured one. `configured` is whether an account is set. Absent when no account is set and
no decision was ever answered. Unanswered decisions are not counted: counting them would mean
reading up to 80 MiB of decision log on every refresh (`anneal calibrate` reads it).

### `context`

The newest session's context usage, as `anneal statusline` (`context-usage.json` in the anneal
home) or the mod, through `anneal snapshot write` (`context-usage-mod.json`), last saved it:
`used_tokens` (null before Claude Code reports usage), the auto-compact `window` and where it
came from, `percent` to one decimal, and `level`, the context monitor's threshold reached
(`warn` at 60%, `stop` at 75%, null below). Of the two files' newest readings the fresher is
shown, by the time it was saved, the mod's on a tie (a reading saved before readings carried a
time is older than any that does); the context monitor judges the same way. The files key
sessions by a hash of their id, so the reading is the newest of any session, not necessarily
this one's. Absent until the status line or the mod saves a reading. When one file cannot be
read, the other's reading is shown and the `status/unreadable:context` finding names the one
that could not be.

### `ledger_tail`

The newest `--tail` events of the ledger, oldest first: `event`, `type`, `summary` (its type
and what it is about, as the handoff note writes it: `task_done task:T1`), `actor` and `ts`.
`[]` for an empty ledger.

## Cost

`--fields phase,plan,gates` reads the latest `mode_set` of the worktree, the latest
`plan_approved` of the plan, the plan file and the newest 10 blocking events: each read asks
the store for at most a few events, newest first. A measurement, reported here and not gated
(release build, Linux container, 2026-10-19; median of 20 runs, wall time of the whole process):

| Ledger | `--fields phase,plan,gates` | all sections |
|---|---|---|
| 12 events | 24 ms | 25 ms |
| 10,002 events | 21 ms | 24 ms |

The time does not grow with the ledger: it is starting the process, opening the store and the
git calls that find the ledger, the worktree and `HEAD`. Method: a scratch repository in build
mode with an approved plan (`anneal plan approve`, `anneal mode build`), then N events appended
with `anneal ledger append` (one `gate_blocked` in ten, the rest `task_done`), then
`anneal status --json --fields phase,plan,gates` and `anneal status --json` each run 20 times.
