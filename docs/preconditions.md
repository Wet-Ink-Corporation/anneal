# The preconditions hook and `anneal mode`

`anneal hook preconditions` runs before every Edit and Write tool call. It is anneal's core build
gate: while a worktree is in an anneal mode, the agent cannot edit source until the work that
mode needs first exists. With no mode it does nothing, so plain usage is never blocked.

## Modes

The mode is recorded in the ledger by `anneal mode`, run in the worktree:

```text
anneal mode build --plan <plan.md> --tree <design-tree.json> [--session <id>] [--store <file>]
anneal mode diagnose [--session <id>] [--store <file>]
anneal mode off [--session <id>] [--store <file>]
```

Each appends one `mode_set` event (registry v4, [ledger.md](ledger.md)): `mode` (`build`,
`diagnose` or `off`), `clean`, and for `build` the `plan` and `tree`. The envelope's `worktree`
is the worktree's top-level directory as `git rev-parse --show-toplevel` prints it, so the event
is tagged `worktree:<that path>`. The worktree's mode is its latest `mode_set`; `off`, or none
at all, means no mode. Every linked worktree has its own mode, although they share one ledger.

- `--plan` and `--tree` must be regular files inside the worktree. They are recorded relative to
  the worktree root, separated by `/`, so the hook finds them from any directory.
- `clean` is whether `git status --porcelain` printed nothing when the mode was entered:
  untracked files count, ignored ones do not. A dirty tree is still recorded, with
  `clean: false` and a `mode/dirty` finding, and exit 0; the hook then denies source edits until
  build mode is entered again on a clean tree.
- Exit 0 when the mode is recorded. Outside a git worktree, or with a `--plan` or `--tree` that
  is not a file inside the worktree, it is a usage error (2) and nothing is recorded.

The `build` and `diagnose` skills (#24, #25) enter these modes; `anneal repro record` (#25)
records the red reproduction diagnose mode waits for.

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

Source edits need a `repro_red` event recorded after the latest `mode_set` that entered diagnose
mode: a recorded reproduction that failed. A reproduction recorded before, for an earlier
session, does not count, so entering diagnose mode again needs a new one.

### Always allowed

In either mode, these paths are never source and are always allowed:

- test files, as `anneal floor-guard` reads them: under a `tests` directory, or named
  `*_test.*`, `*.test.*`, `*.spec.*` or `test_*.py`;
- the build's plan, its spec and its design tree;
- anything under `.temper/`, `.anneal/`, `tmp/` or `scratch/` at the worktree root;
- prose documents: `*.md`, `*.txt` and `*.rst`;
- any path outside the worktree.

Paths are compared after they are made relative to the worktree root: `\` reads as `/` (Windows
paths and drive letters work), `.` and `..` are taken lexically, symlinks in the existing part
of the path and in the root are resolved, and ASCII letters match in any case, as the guard's
protected paths do (APFS and NTFS are case-insensitive by default). So `tests/../src/main.rs` is
source.

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

A crash, a timeout or output Claude Code cannot read blocks the edit (`onFailure: "block"`), and
with no binary the launcher exits 2 for `--fail-closed`. The hook itself denies what it cannot
judge:

| Failure | Result |
|---|---|
| stdin is not `PreToolUse` JSON, is larger than 1 MiB, or has no `tool_input.file_path` | deny, `bad-input` |
| the ledger exists but cannot be opened or read; git fails other than "not a repository"; the path cannot be resolved | deny, `unjudged` |
| build mode recorded without a plan or tree (an event appended by hand) | deny, `mode` |
| the plan or design tree cannot be read | deny, `plan` or `grill` |
| the decision cannot be written | exit 2 with one `ERROR` line |

Only these mean "no mode" and allow: no ledger file (the hook never creates one), a directory
outside any git repository, and no `mode_set` (or `off`) for the worktree.

It stays inside its 5 s budget as the guard does: git gets 1 s for each of its two calls (the
ledger's location, the worktree root) and each ledger statement waits at most 500 ms for
another process's lock.

## Known gaps

- The matcher is `Edit|Write`, as the story specifies. Claude Code's `MultiEdit` and
  `NotebookEdit` tools, and Bash commands that write files, are not gated by this hook.
- Paths are judged lexically past the part that exists, so a symlink created later inside the
  worktree is not followed.
