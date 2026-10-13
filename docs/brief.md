# Briefs, review packages and fix-loop rounds

No brief, no spawn. A subagent gets files, not pasted text: a brief that says what to do and
how the work is judged, and, for a reviewer or a fixer, a review package with the diff and that
same contract. Fix loops are bounded by a counter in the ledger, not by an agent's memory.

## The nine fields

Every brief has these nine sections, in this order. The set lives in one table in code
(`crates/anneal-cli/src/brief/fields.rs`); renaming a field there renames it for
`brief build`, `lint brief`, the brief hook and the review package.

| Field | Key (`--field`) | Purpose |
|---|---|---|
| Goal | `goal` | The outcome the subagent delivers, in one or two sentences. |
| Repository | `repository` | The worktree it works in and the base its change starts from. |
| Owned scope | `owned-scope` | The files, modules or commands it owns and may change. |
| Do not touch | `do-not-touch` | The paths, interfaces and decisions it must leave as they are. |
| Invariants | `invariants` | The data shapes and rules that must still hold when it is done. |
| Success criteria | `success-criteria` | How the caller judges the work finished. |
| Verification | `verification` | The commands it runs to check its work, and what they must show. |
| Edits allowed | `edits-allowed` | Whether it may change files (yes or no), with the no-revert rule. |
| Return contract | `return-contract` | What its final report must hold. |

The set is re-authored from the public pstack's worker brief (`kashyab12/uni-pstack`,
`pstack/references/delegation.md`; see `CREDITS.md`). The story asked for the brief fields of
pstack's `orchestrate.md`, which was out of reach, so this set is a default recorded as an open
decision for Ryan, not a copy of `orchestrate.md`.

## `anneal brief build`

```text
anneal brief build --ticket <id> --base <ref> [--field <key>=<text>]... [--waits-on-ci]
```

It writes two files under the worktree root, replacing earlier ones:

- `.anneal/briefs/<id>.md`, the brief: a title, the review package's path, the nine fields,
  then a `## Volatile` section with the base and head commits, `cacheTtl: 1h` when
  `--waits-on-ci` is passed (a fix-loop subagent that waits on CI), and the build time. What
  stays the same comes first and what changes on every build last, so a prompt that starts with
  the stable part keeps its cache.
- `.anneal/briefs/<id>.review.md`, the review package: the contract (the nine fields) and
  `git diff <base>...HEAD` in a fenced block, then the same volatile section. A diff over 4 MiB
  is not included; the package names the `git diff` command that shows it instead, and the
  envelope sets `data.diff_omitted`.

`--ticket` is a plain id used for file names: an issue number (`36`) or a dag ticket id
(`003-auth`), 1 to 64 ASCII letters, digits, `.`, `_` or `-`, not starting with `.`. Letters are
lowered (`T1` is `t1`), since names differing only in case share one file on case-insensitive
file systems (APFS, NTFS); the round counter uses the same lowered name. A name ending in
`.review` (its brief would be another ticket's review package) and a Windows device name (`con`,
`prn`, `aux`, `nul`, `com1` to `com9`, `lpt1` to `lpt9`, in any case and before any `.`) are
refused. Nothing is fetched from a tracker.

A field not given is written as an HTML comment naming its purpose, which `lint brief` does not
count as content, except `repository` (the worktree root and the base) and `return-contract`
(files changed, what was done, the verification and its result, risks or blockers). The
`edits-allowed` text is followed by the no-revert rule: never revert, reset or overwrite a
change you did not make. `data.empty_fields` lists the fields still to fill.

The range guard is rooted at the base: the base must name a commit, `HEAD` must name one, and
the base must be an ancestor of `HEAD`. Otherwise it exits 2 with one `brief/range` finding and
writes nothing. A usage error (a bad ticket or field, outside a worktree) also exits 2 but with
nothing on stdout.

The files are written only when `.anneal/briefs` resolves to the resolved worktree root's own
`.anneal/briefs`: a symlinked `.anneal` or `briefs` leading elsewhere exits 70 with one `ERROR`
line naming where it leads, and nothing is written there. The directories are made one at a
time, `.anneal` then `briefs`, each checked before the next is made, so a symlinked `.anneal`
does not get an empty `briefs` directory where it leads. git failing to resolve the range
(`resolving the range`) or to read the diff (`reading the diff`) is also 70.

## `anneal lint brief <file>`

Exit 0 when each of the nine fields is one `## <heading>` section with content, else 1 with one
finding per problem: `field-missing` (no section; line 0), `field-empty` (only blank lines,
comments or fenced text) and `field-repeated` (a second section, at its line). Headings match
in any ASCII letter case; other sections, such as `## Volatile`, are allowed.

## The brief hook

`anneal hook brief` is a fail-closed `PreToolUse` hook on the Agent tool (see
[hooks.md](hooks.md)). A spawn's prompt names its brief on one line:

```text
Brief: .anneal/briefs/36.md
```

The path is relative to the worktree root, under `.anneal/briefs/`, with no `..`, `.`, `\` or
absolute form, and it must resolve, symlinks included, to a regular file of at most 256 KiB
inside that directory that passes `lint brief`. A spawn from inside a subagent (its input has
`agent_id`) is denied unless its `agent_type` is an orchestrate lead (one that names no
`agent_type` is denied without reading the list):

```json
{"orchestrate": {"leads": ["orchestrate-lead"]}}
```

in `.anneal/config.json`; the list replaces the default, `["orchestrate-lead"]`, and holds at
most 64 names, each 1 to 128 bytes with no control character. An empty list lets no subagent
spawn. A setting that breaks these, or a file that cannot be read, denies every nested spawn as
unjudged, naming the problem (for a name, its index). A plugin's agent reports a plugin-scoped type (`anneal:orchestrate-lead`), so
list that form for one.

An agent cannot add itself to the list: the guard denies Bash writes to `.anneal/config.json`
(and removing `.anneal`), and the preconditions hook denies the Edit and Write tools on it in
every mode and with none (`preconditions.settings`), so a human edits it. The hook's matcher is
`Edit|Write`, so `MultiEdit` and `NotebookEdit` on it are left to Claude Code's permissions, as
for every file the hook gates (`docs/preconditions.md`).

## Standing orders

`anneal hook standing-orders` (`SubagentStart`, fail open) adds `.anneal/standing-orders.md`
at the worktree root to every subagent's context. It is silent when the file is missing and
injects nothing, with one `ERROR` line as `systemMessage`, when it is over 16 KiB, not a
regular file, or resolves outside the worktree's own `.anneal/` (a symlinked `.anneal`). Claude Code moves context over 10,000 characters to a file and passes a preview,
so keep orders meant to be read whole under that.

## Fix-loop rounds

```text
anneal round start --ticket <id> [--session <id>]
anneal round status --ticket <id>
```

A ticket's round count is the highest `round` among its `round_started` events (registry v8:
`{"type": "round_started", "ticket": "36", "round": 2}`), so it survives compaction and
restarts. `start` appends the next round, first writer wins: when another `start` of the same
ticket recorded that round first, it reports that round (`data.joined`) and appends nothing;
when only other tickets' rounds came in between, it reads again, up to 4 times, then fails with
exit 70. Rounds are found by type, not tagged by ticket, so both read the `round_started`
events of every ticket, at most 4,096; a ledger holding more fails with exit 70 rather than
counting a cut history (a tag key for tickets would lift this; it is a registry decision).
`status` reads and never creates a ledger. `data.next` says what the round does:

| Round | `next` | What the orchestrator does |
|---|---|---|
| 1 to 3 | `same-agent` | Send the review package back to the same agent. |
| 4 and 5 | `fresh-agent` | Spawn a fresh agent one model tier up through `context: fork` with a named subagent; never `model:` in frontmatter. |
| after 5 | `park` | Stop the fix loop and park the ticket to weigh-in with `anneal orchestrate park` ([orchestrate.md](orchestrate.md)). |

Both exit 0 while a round may run and 3 (`park`) once five rounds ran; `start` then records
nothing. For `status`, `next` is the round after the count; for `start`, the round it recorded.
