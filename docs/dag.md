# The ticket graph: `anneal dag`

Work is cut into tickets that each fit one fresh context, joined by "blocked by" edges.
`anneal dag check` proves the edges form a graph you can work through: no cycle, and no
reference to a ticket that does not exist. `anneal dag frontier` lists the tickets that are
ready now, each with a token estimate, so `/orchestrate` can fan out over independent work
(`anneal orchestrate fan-out`, [orchestrate.md](orchestrate.md)) and split anything too large
for one context.

```text
anneal dag check    [--tracker markdown|github] [--dir PATH] [--repo OWNER/NAME] [--issue N]
                    [--plan FILE]
anneal dag frontier [same tracker flags] [--budget TOKENS]
```

`--tracker` defaults to `markdown`. `--dir` (markdown, default `tickets`) is relative to the
working directory. `--repo` is required for `github`, and `--issue` is optional. Passing a
flag that belongs to the other tracker is a usage error (exit 2, nothing on stdout).

## Trackers

### Local markdown

One ticket per `<dir>/*.md` file. The ticket's id is the file stem (`003-auth.md` is
`003-auth`). `README.md` (in any case) is not a ticket, and neither is anything that is not a
regular `.md` file. Frontmatter is optional and uses the same YAML subset as `SKILL.md`:

```markdown
---
title: Add the auth middleware
status: open
blocked_by: [001-db, 002-config]
files: [src/auth.rs, src/main.rs]
---
The body: what to do, and which `src/files.rs` it touches.
```

| Key | Meaning |
|---|---|
| `blocked_by` | Ids that must be done first. It can be a flow list (`[001, "002"]`), a block list (`- 001` lines) or a comma list (`001, 002`). A ref must match a file stem exactly. |
| `status` | `done` or `closed` (in any case) means done. Anything else, or no status, means open. |
| `title` | Defaults to the first `# ` heading, then to the id. |
| `files` | Files the ticket will touch, in the same list forms. The estimate uses them. |

Other keys are ignored. A ticket whose frontmatter cannot be read stops the command with exit 70
and names `file:line`. Limits: 10,000 tickets, and 1 MiB per file.

### GitHub Issues

Read-only, through `gh api --method GET` (REST). `gh` must be installed and logged in.

- With `--issue N`, the tickets are the sub-issues of issue N
  (`repos/{owner}/{repo}/issues/{N}/sub_issues`).
- Without it, the tickets are every issue in the repository, open and closed
  (`repos/{owner}/{repo}/issues?state=all`). Pull requests are skipped.
- A ticket's id is its issue number, and it is done when the issue is closed.
- Its blockers are the `#n` references after `Blocked by` in the issue body. The match ignores
  case and allows an optional `:` and markdown emphasis. All of these work:
  `Blocked by #10`, `**Blocked by:** #10`, `Blocked by #1, #2 and #3`. A cross-repository
  reference (`owner/repo#4`) is not read. `Blocked by` counts only as a whole word: at the
  start of the body, or after a character that is not a letter, digit or `_`. So
  `Unblocked by #5` names no blocker. It also does not count right after `no longer ` or
  `not ` (`no longer blocked by #12`, `Not blocked by #13`).
- A blocker outside the tickets that were read, such as a story from an earlier release or a
  pull request, is fetched on its own with `gh api repos/{owner}/{repo}/issues/{n}`. It
  becomes an **external** ticket:
  - **Closed** (a merged or closed pull request counts the same way): the blocker is
    satisfied.
  - **Open**: the dependant is not ready. `frontier` lists the blocker under
    `external_blockers` with the tickets it holds up.
  - **Not found or deleted** (`gh` reports HTTP 404 or 410): the reference stays **dangling**, and the command
    exits 2.

  External tickets are leaves: their own blockers are not followed. They are never counted,
  checked or listed as ready, so cycle detection is unchanged.

Bounds: each call has a 30 s timeout and keeps at most 16 MiB of output. Pages hold 100 issues,
and the reader stops after 20 pages. If all 20 pages come back full, the command fails rather
than return part of the graph; narrow it with `--issue`. At most 100 distinct outside
blockers are fetched, one call each. If there are more, the command fails before fetching any
of them. Any other `gh` failure (not installed, not logged in, rate limited, a 404 on the
issues read) exits 70 with `gh`'s message.

### The tracker adapter trait

Every tracker implements `anneal_cli::dag::tracker::Tracker`:

```rust
pub trait Tracker {
    /// The tracker's name as `--tracker` spells it.
    fn name(&self) -> &'static str;
    /// Reads every ticket in scope.
    fn read(&self) -> Result<Vec<Ticket>, TrackerError>;
    /// Reads every ticket in scope plus the outside tickets they are blocked by.
    /// The default returns no outside tickets.
    fn read_with_external(&self) -> Result<Tickets, TrackerError> { /* ... */ }
}
```

`Tickets` holds `in_scope` and `external`. Only a tracker whose scope is part of a larger
tracker overrides `read_with_external`. GitHub does, because `--issue` reads only the
sub-issues of one issue. It must look up each outside reference and leave unknown ones out, so
that they stay dangling.

A `Ticket` has an `id`, `title`, `body`, `blocked_by` ids, explicitly listed `files` and a
`status` (`Open` or `Done`). An implementation must:

- be read-only;
- return unique ids, with `blocked_by` written in the same ids. Keep references to tickets that
  do not exist, because the graph reports them as dangling;
- map its own states onto `Open` and `Done`, using `Done` only for finished work;
- bound what it reads: at most `MAX_TICKETS` (10,000) tickets, capped sizes, and a timeout on
  any call outside the process. It fails rather than truncate.

Order does not matter, because the graph sorts. To add ADO or Jira (#45), add a module under
`dag::tracker`, add its error as one more `TrackerError` variant, and add one more
`--tracker` value. The graph, the estimate and the output stay unchanged. The GitHub reader
shows the pattern for remote trackers: it calls a small port (`GhApi`), so tests answer from
recorded JSON (`crates/anneal-cli/tests/fixtures/dag/github/`) with no network.

## Order

Every list (tickets, dangling references, the frontier) is in id order:

- An id made only of ASCII digits that fits in a `u64` is a number. Numbers come first, by
  value, so `2` comes before `10`.
- Every other id follows, by bytes.
- Two ids with the same value (`01` and `1`) order by bytes.

Two runs over the same tickets print byte-identical output.

## `dag check`

| Exit | Meaning |
|---|---|
| 0 | Every reference resolves and there is no cycle. |
| 1 | There is a cycle. One cycle is named, in order. |
| 2 | A reference dangles: some ticket is blocked by an id that is not a ticket. Every pair is named. |
| 3 | Only with `--plan`: the graph passes, but the tickets disagree with the plan (below). |
| 70 | The tickets could not be read. |

**Dangling comes first.** When both problems occur, the exit code is 2, and the envelope still
lists both, because a cycle that runs through a missing ticket cannot be judged.

The reported cycle is deterministic. A depth-first search starts from tickets in id order and
follows blockers in id order. The first edge back onto the current path closes the cycle, which
is then rotated to start at its smallest id. Each id is blocked by the next, and the last by
the first. The search is iterative, so a very long chain cannot overflow the stack. A ticket
blocked by itself is a cycle of one.

`data` is `{tracker, tickets, done, edges, dangling: [{ticket, missing}], cycle: [ids]}`.
`tickets` and `done` count only tickets in scope, while `edges` also counts edges to external
tickets. The
findings are `dag/dangling:<ticket>-><missing>` for each dangling reference and
`dag/cycle:<a>-><b>...` for the cycle.

## `dag check --plan`

Nothing turns a plan's tasks into tickets: the `Blocked by:` lines of `plan.md` are copied
into the tickets by hand, so a plan approved with one dependency graph could be built from
tickets with another. `--plan <FILE>` also compares the plan's tasks with the tickets, after
the graph check.

- **The mapping is by id.** A ticket answers the task whose id is the ticket's id, exactly
  and in the same letter case. For markdown tickets the id is the file stem, so
  `tickets/T1.md` answers `### T1: ...`, as the recorded dry run
  (`evals/fixtures/dry-run/`) does. For GitHub the id is the issue number, so the plan's task
  ids must be the issue numbers. Titles are not compared.
- **The plan is read as `anneal lint plan` reads it** (same reader and 1 MiB cap; see
  [plan.md](plan.md)). The path is relative to the working directory. A missing plan, a
  directory or a file over the cap is a usage error (exit 2, one `ERROR plan:` line, nothing
  on stdout); a read failure exits 70.
- **Blockers are compared as sets**: order and repeats do not matter. Only the blockers that
  are tasks of the plan count, on both sides. A blocker from outside the plan, such as
  another team's ticket or a release, belongs in the tracker alone (the `plan` skill's
  `references/slice.md`), so it is never a disagreement.
- **An outside blocker's own ticket is not a ticket with no task.** With the markdown
  tracker an outside blocker such as `REL-5` must exist as `tickets/REL-5.md`, or the graph
  dangles. A ticket that is no task of the plan but blocks a task's ticket, directly or
  through other such tickets (`REL-5` blocked by `REL-4`), is an outside blocker: it is
  not reported, and its own blockers count only as the order it puts on the plan's tasks: a
  task whose ticket is blocked by `R1`, itself blocked by task `T1`, waits for `T1`, so the
  task's `Blocked by:` must name `T1`. Any other ticket in scope that is no task is
  `dag/plan/no-task`.
- **With GitHub, pass `--issue`.** Without it every issue of the repository, open or closed, is in
  scope, so each unrelated issue is a `dag/plan/no-task` finding. With `--issue N` the scope
  is the plan's parent issue's sub-issues, and blockers outside it are fetched as external
  tickets, which are never in scope.
- **The plan's own outside blockers.** A plan task `Blocked by:` an id that is no task of the
  plan is a `lint plan` `blockers` finding (outside blockers belong in the tracker), so it
  counts in `unread` rather than being compared.

Findings, after the graph's, each in id order:

| Finding | Meaning |
|---|---|
| `dag/plan/unread:<file>` | `lint plan` finds a problem that may hide a task or blocker from the comparison: a `tasks` or `task-fields` finding, a `blockers` finding naming a blocker that is no task of the plan, or findings it dropped past its cap of 1,000. Run `anneal lint plan`. |
| `dag/plan/no-ticket:<id>` | A task of the plan has no ticket. |
| `dag/plan/no-task:<id>` | A ticket in scope is neither a task of the plan nor an outside blocker of one. |
| `dag/plan/blockers:<id>` | The task's ticket waits for other plan tasks than the task's `Blocked by:` names: its blockers that are tasks, and the tasks it reaches through outside blockers (`T2` blocked by `R1`, itself blocked by `T1`, waits for `T1`). The message names both lists. |

The exit code is the graph's when its check fails (2 or 1; the plan's findings are still
listed), else 3 when there is any plan finding, else 0. `data` gains
`plan: {file, tasks, no_ticket: [ids], no_task: [ids], blockers: [{id, plan, ticket}], unread}`,
where `file` is the plan as given and `unread` counts the `lint plan` findings above (and
those it dropped). At most 1,000 disagreements are listed, in data and as findings, the
first in finding order (tasks with no ticket, then tickets with no task, then blockers, each
in id order); `plan.omitted`, present only when some were left out, counts the rest. The
plan's spec is not looked for. Without `--plan`, `data` has no `plan` key and `dag check` is
unchanged. `dag frontier` takes no `--plan`.

## `dag frontier`

`frontier` runs the same check first. If the check fails, the frontier lists no tickets and
exits with the check's code and findings. Otherwise it lists every **ready** ticket, in id
order: open, with every blocker done. Done tickets are never ready.

`data` is `{tracker, budget, ready, over_budget, tickets, external_blockers}`. Each ticket is
`{id, title, blocked_by, over_budget, estimate: {tokens, body_tokens, files}}`. Each external
blocker is `{id, title, blocks: [ids]}`: an open ticket from outside the scope, with the open
tickets in scope that wait for it. The list is empty for markdown.

### The token estimate

The estimate is a deterministic heuristic that judges whether a ticket fits one context:

```text
tokens = ceil(body characters / 4) + the cost of each named file
```

- **Named files** are the ticket's `files` list, counted as written, followed by every
  backticked path in its body. A backticked path is one word with a letter and either a `/` or
  a known file extension, such as `` `src/a.rs` ``, `` `docs/dag.md` `` or
  `` `Cargo.toml:12` ``. A trailing `:line` is dropped. These are not paths: options (`--x`),
  URLs, phrases, globs (a word holding `*`, `?` or `[`, such as `` `tickets/*.md` ``), command
  names and absolute paths (any word starting with `/`, such as `` `/orchestrate` `` or
  `` `/usr/lib/x.rs` ``, so it is not sized and costs nothing; a `files:` entry is still counted
  as written), and dotted identifiers with
  no `/` and no known extension (`` `grill.confirmed` ``, `` `data.needs_clarification` ``).
  The known extensions, in any letter case, are `bash bat c cc cfg cjs cmd conf cpp cs css csv
  go gql graphql h hpp html ini java js json jsonl jsx kt lock lua md mjs php proto ps1 py rb
  rs rst scss sh sql svg swift toml ts tsv tsx txt xml yaml yml zsh`. Each file counts once,
  and at most 64 are counted.
- **A file's cost** is `ceil(size / 4)` tokens, capped at 25,000, when it is a regular file
  under the working directory (for every tracker, also markdown with `--dir`: a path in a
  ticket is read relative to where `anneal` runs, not to the tickets directory, so run it from
  the directory the tickets' paths are written against): a relative path with no `..`, not a
  symlink, and with its canonical path inside the canonical working directory, so a symlinked
  directory cannot lead outside it. Any other named file (missing, outside the root, or a
  directory) costs 2,000 tokens. Files are only sized, never read.
- The sum saturates and does not overflow.

`--budget` (default 100,000 tokens) is the single-context budget. A ticket whose estimate
exceeds it gets `over_budget: true` and a `dag/over-budget:<id>` finding. The flag does not
change the exit code: a large ticket is a planning signal, not a broken graph.
