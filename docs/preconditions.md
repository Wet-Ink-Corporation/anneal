# The preconditions hook and `anneal mode`

`anneal hook preconditions` runs before every Edit, Write, MultiEdit and NotebookEdit tool call.
It is anneal's core build gate: while a worktree is in an anneal mode, the agent cannot edit
source until the work that mode needs first exists. With no mode it does nothing, so plain usage
is never blocked.

Every file a call names is judged: `tool_input.file_path` (Edit, Write, MultiEdit),
`tool_input.notebook_path` (NotebookEdit) and the `file_path` of any MultiEdit edit that has one.
The tool's own field is required; the first file denied denies the call. A call naming more than
16 distinct files is denied as bad input.

## Modes

The mode is recorded in the ledger by `anneal mode`, run in the worktree:

```text
anneal mode build --plan <plan.md> --tree <design-tree.json> [--session <id>]
anneal mode diagnose [--session <id>]
anneal mode off [--session <id>]
```

Each appends one `mode_set` event (registry v4, [ledger.md](ledger.md)): `mode` (`build`,
`diagnose` or `off`), `clean`, and for `build` the `plan` and `tree`. The envelope's `worktree`
is the worktree's key: its top-level directory as `git rev-parse --show-toplevel` prints it,
with `/` separators, and in ASCII lowercase on Windows and macOS, whose file systems ignore
letter case by default (so every spelling of the directory is one worktree). The event is tagged
`worktree:<key>`. The worktree's mode is its latest `mode_set`; `off`, or none at all, means no
mode. Every linked worktree has its own mode, although they share one ledger.

There is no `--store`: the mode goes to the ledger the hook reads, resolved as every command
resolves it without the flag (`$ANNEAL_STORE`, else the git common dir, else `.anneal/` in the
folder; [ledger.md](ledger.md)).

- `--plan` and `--tree` must be regular files inside the worktree. They are recorded relative to
  the worktree root, separated by `/`, so the hook finds them from any directory.
- `clean` is whether `git status --porcelain` printed nothing when the mode was entered:
  untracked files count, ignored ones do not. A dirty tree is still recorded, with
  `clean: false` and a `mode/dirty` finding, and exit 0; the hook then denies source edits until
  build mode is entered again on a clean tree.
- Exit 0 when the mode is recorded. Outside a git worktree, or with a `--plan` or `--tree` that
  is not a file inside the worktree, it is a usage error (2) and nothing is recorded.

The `build` and `diagnose` skills (#24, #25) enter these modes; `anneal repro record` (below)
records the red reproduction diagnose mode waits for.

## Recording a reproduction

```text
anneal repro record [--id <task or bug id>] [--timeout <secs>] [--session <id>] -- <command>
anneal repro leftovers
```

`repro record` runs the command as `anneal task done` runs a test command: as given, without a
shell, in the working directory, with stdin closed and its output on anneal's stderr, killed
after `--timeout` seconds (default 1800). Only a red run appends `repro_red` (`task` is `--id`,
default `repro`; `test` is the command), with the envelope's `worktree` set to the key `anneal
mode` writes, so it counts for this worktree only. Like `anneal mode` it has no `--store`.

| Run | Recorded | Exit |
|---|---|---|
| exits non-zero, or is killed by a signal | `repro_red` | 0, `data.recorded: true` |
| exits 0: it reproduces nothing | nothing, and no ledger is opened | 3, finding `repro/not-red:<id>` |
| runs past `--timeout`: a hang is not a reproduction | nothing, and no ledger is opened | 4, finding `repro/timed-out:<id>` |
| cannot be started | nothing | 2, one `ERROR` line |

Outside a git worktree, or with an `--id` or `--session` the ledger cannot tag, it is a usage
error (2) and runs nothing. `data` reports `id`, `command`, `worktree`, `outcome` (`red`,
`green` or `timed_out`), `exit`, `duration_ms`, `timeout_secs`, `recorded` and, when recorded,
`ledger` (`store`, `source`, `position`, `event`).

`repro leftovers` lists every line that holds a debug tag, `[DEBUG-` then an id of 1 to 64 ASCII
letters, digits, `_` or `-`, then `]`, in the files git knows of in the worktree (tracked, plus
untracked files git does not ignore; prose included). Text that spells the pattern with a
placeholder, as this page does, is not a tag. Lines are read as bytes, so a file that is not
UTF-8 is scanned as it is. Each file is read up to 1 MiB and the whole scan up to 64 MiB; a file
over either is listed in `data.skipped`, not read, and running out of the total sets
`data.over_budget`. A file name that is not UTF-8 is opened by its raw bytes on unix and reported
with U+FFFD in place of the bad bytes; elsewhere such a name is listed in `data.skipped`. It
exits 1 with one finding per line (`<file>:<line>: ...`,
at most 1,000, the rest counted in `data.omitted`) and 0 when there are none. The same scan is
`anneal_cli::repro::debug_leftovers`, for the Stop hook (#26).

## What the hook checks

### Build mode

Source edits need all of these, checked in this order; the first that fails is the denial:

| Precondition | Satisfied when | The command that satisfies it |
|---|---|---|
| spec | the plan's `Spec:` line links a spec that passes `anneal lint spec`, with `CONTEXT.md` and `constitution.md` at the worktree root when they exist | `anneal lint spec <spec>` |
| grill | one of the latest 256 `grill_confirmed` events carries the `sha256:` hash of the design tree's current bytes | `anneal grill confirm --tree <tree> --quote "<the human's words>"` |
| clean | the tree was clean when build mode was entered | `anneal mode build --plan <plan> --tree <tree>` on a clean tree |
| plan | `anneal plan check` would exit 0: the latest approval of the plan has its current hash | `anneal plan approve <plan>` |

The plan's approvals are looked up by the plan's path relative to the worktree root, which is
the id `anneal plan approve <plan>` records when run from the root. The grill confirmation is
matched by hash, so any path spelling of the tree counts.

### Diagnose mode

Source edits need a `repro_red` event tagged with the edited worktree's key, recorded after the
latest `mode_set` that entered diagnose mode: a recorded reproduction that failed, in this
worktree (`anneal repro record`). A reproduction recorded before, for an earlier session, does
not count, so entering diagnose mode again needs a new one. Neither does one recorded in another
worktree of the same repository, nor a `repro_red` without a worktree (appended by hand with
`anneal ledger append`). This is the `repro_red` part of #80, done in #25.

### Always allowed

In either mode, these paths are never source and are always allowed:

- test files, as `anneal floor-guard` reads them (under a `tests` directory, or named
  `*_test.*`, `*.test.*`, `*.spec.*` or `test_*.py`), and Rust test module files named
  `tests.rs` or `test.rs`;
- the build's plan and design tree, as recorded at entry;
- anything under `.temper/`, `.anneal/`, `tmp/` or `scratch/` at the worktree root;
- prose documents: `*.md`, `*.txt` and `*.rst`. The spec is editable as one of these; it is not
  exempt by name, since the plan's `Spec:` line is itself editable and naming a source file
  there would otherwise unlock it;
- any file in no git repository.

One file is denied in every mode, and with no mode or no ledger: `.anneal/config.json`
(`preconditions.settings`), anneal's own settings (the orchestrate leads the brief hook trusts,
the merge gate's keys, `check_fast`). It is judged before git or the ledger is asked, as written
after `.` and `..`, as the file resolves when it exists and through a symlinked directory, in any
ASCII letter case, with trailing dots and spaces and an alternate data stream (`:...`) taken off
the names; a `.anneal/config.json` in any directory counts. The reason tells the user to edit it
themselves.

A path is judged where the OS writes it, not by its name. It is resolved one component at a
time: a symlink is followed to its target (at most 40 links, else the edit is denied as
`unjudged`), so `..` after a symlink climbs from the link's target and a dangling symlink leads to
the file it would create; an existing component is resolved in full; the part that does not
exist is taken lexically (`.` dropped, `..` the parent). With `tmp/link -> ../src/deep`, a write
of `tmp/link/../app.py` is `src/app.py`, and with `tmp/new -> ../src/new.py`, a write of
`tmp/new` is `src/new.py`: both are source.

A tool may also take `..` lexically before it hands the path to the OS, and then writes
elsewhere. So a path is judged in both readings, where the OS writes it and where its lexical
normalisation (`.` dropped, `..` the parent of the name before it, then resolved as above)
lands, and either reading's denial denies. With `src/link -> ../tmp/sub`, a write of
`src/link/../x.py` is `tmp/x.py` to the OS (scratch) but `src/x.py` to such a tool (source), so
in a mode it is denied wherever `src/x.py` is. When both readings land in the same place it is
judged once.

The worktree that judges an edit is the edited file's own: git is asked from the longest
existing ancestor directory of where the file lands, not from the session's working directory.
So a session running above the repository, or in a sibling linked worktree, cannot write into a
gated worktree, and neither can a symlink in another repository that leads into it. A file in
another repository is judged by that repository's ledger and that worktree's mode.

Paths are compared after they are made relative to the worktree root: `\` reads as `/` (Windows
paths and drive letters work), symlinks are resolved in the path as above and in the root, and
ASCII letters match in any case, as the guard's protected paths do (APFS and NTFS are
case-insensitive by default). So `tests/../src/main.rs` is source.

## Output

- **Allowed:** no output and exit 0.
- **Denied:** Claude Code's `PreToolUse` deny decision on stdout and exit 0. The reason is one
  `ERROR` line naming the missing precondition and the command that satisfies it:

  ```json
  {"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny",
   "permissionDecisionReason":"ERROR build mode: the plan docs/plans/x.md is not approved; have the human review it, then run `anneal plan approve docs/plans/x.md`"}}
  ```

  The hook also appends a `gate_blocked` event with `point` `preconditions.<check>` (`spec`,
  `grill`, `clean`, `plan`, `mode`, `repro`, `bad-input` or `unjudged`), the reason, actor
  `hook` and the session id. If the ledger cannot take the event, the edit is still denied and
  the reason ends with `(the block was not recorded in the ledger: ...)`.

## Failing closed

The entry in `plugin/hooks/hooks.json`:

```json
{ "type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/libexec/anneal",
  "args": ["hook", "preconditions", "--fail-closed"], "timeout": 5, "onFailure": "block" }
```

A crash, a timeout or output Claude Code cannot read blocks the edit (`onFailure: "block"`). With
no binary at all the launcher exits 0 and the hook is off, as the session start warns. The hook
itself denies what it cannot judge:

| Failure | Result |
|---|---|
| stdin is not `PreToolUse` JSON, is larger than 1 MiB, has no `tool_input.file_path` (`tool_input.notebook_path` for NotebookEdit), or names more than 16 files | deny, `bad-input` |
| the ledger exists but cannot be opened or read; git, asked from the file's longest existing ancestor directory, fails other than "not a repository"; the file has no existing ancestor directory, cannot be resolved (a component that cannot be read, more than 40 symlinks), or is not inside its own worktree once symlinks are resolved; the 3.5 s budget runs out before every file is judged | deny, `unjudged` |
| build mode recorded without a plan or tree (an event appended by hand) | deny, `mode` |
| the plan or design tree cannot be read | deny, `plan` or `grill` |
| the decision cannot be written | exit 2 with one `ERROR` line |

Only these mean "no mode" and allow: a file in no git repository, no ledger file for its
repository (the hook never creates one), and no `mode_set` (or `off`) for its worktree.

It stays inside its 5 s timeout: the whole call, every file and every reading of it, is judged
within 3.5 s of the hook's start (the rest is for recording a denial and process start-up, as
the brief hook does). Git gets at most 1 s for each call, and never more than is left; each
worktree a call reaches is found once per directory (the worktree root) and read once (the
ledger's location, its mode, and whether its preconditions hold, which are the same for every
file in it); each ledger statement waits at most 500 ms for another process's lock, and never
more than is left. Once the budget is spent the edit is denied as `unjudged` ("no time is left
to judge the edit"), and the denial is not recorded when no time was left to open the ledger.

## Known gaps

- The matcher is `Edit|Write|MultiEdit|NotebookEdit` (#80). Bash commands that write files are
  not gated by this hook.
- Paths are judged lexically past the part that exists, so a symlink created later inside the
  worktree is not followed.
