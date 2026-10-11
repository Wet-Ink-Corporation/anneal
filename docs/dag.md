# The ticket graph: `anneal dag`

Work is cut into tickets that each fit one fresh context, joined by "blocked by" edges.
`anneal dag check` proves the edges form a graph you can work through: no cycle, and no
reference to a ticket that does not exist. `anneal dag frontier` lists the tickets that are
ready now, each with a token estimate, so `/orchestrate` (#37) can fan out over independent
work and split anything too large for one context.

```text
anneal dag check    [--tracker markdown|github] [--dir PATH] [--repo OWNER/NAME] [--issue N]
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

- **Named files** are the ticket's `files` list, followed by every backticked path in its body.
  A backticked path is one word with a letter and either a `/` or an extension, such as
  `` `src/a.rs` `` or `` `Cargo.toml:12` ``. A trailing `:line` is dropped. Options (`--x`),
  URLs and phrases are not paths. Each file counts once, and at most 64 are counted.
- **A file's cost** is `ceil(size / 4)` tokens, capped at 25,000, when it is a regular file
  under the working directory: a relative path with no `..`, not a symlink, and with its
  canonical path inside the canonical working directory, so a symlinked directory cannot lead
  outside it. Any other named
  file (missing, outside the root, or a directory) costs 2,000 tokens. Files are only sized,
  never read.
- The sum saturates and does not overflow.

`--budget` (default 100,000 tokens) is the single-context budget. A ticket whose estimate
exceeds it gets `over_budget: true` and a `dag/over-budget:<id>` finding. The flag does not
change the exit code: a large ticket is a planning signal, not a broken graph.
