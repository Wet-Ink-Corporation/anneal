# Surviving compaction: status line, context monitor, handoff

Long sessions must survive `/compact` and `/clear`. Whatever must outlive them lives on disk,
and a script writes it. Four pieces do this (#38):

| Piece | Runs | Does |
|---|---|---|
| `anneal statusline` | Claude Code's status line, wired once in user settings | prints `anneal ctx 120k/200k 60%` and saves the session's reading |
| `context-monitor` hook | `PostToolBatch`, no matcher, 5 s, fails open | warns once at 60% and stops once at 75% of the auto-compact window, per crossing |
| `handoff` hook | `PreCompact`, no matcher, 30 s, fails open | writes `.continue-here` at the repository root |
| `bootstrap` hook | `SessionStart` | injects `.continue-here`, and a one-time hint while no status line is set up |

The `/handoff` skill (user-invoked) is the decision at a phase boundary: continue, clear, hand
off, a subagent or compact.

## The status line

Plugins cannot set the main status line, so it is set once in `~/.claude/settings.json`; the
README's "Status line" section has the setting. `anneal statusline` reads Claude Code's
status-line input (at most 1 MiB) and uses `session_id` and `context_window`:

- **Tokens used** = `context_window.used_percentage` of `context_window.context_window_size`
  (the percentage is always of the model's full window). `null` early in a session and after a
  compaction until the next response: the line shows `-` and the reading saves no tokens.
- **Window**: `CLAUDE_CODE_AUTO_COMPACT_WINDOW`, read as Claude Code reads it (its leading digits,
  clamped to 100,000..=1,000,000 and to the model's window); else the model's
  `context_window_size`; else 200,000 tokens. The `autoCompactWindow` setting and `/autocompact`
  are not read; set the variable to have the monitor follow them.
- **Thresholds** are absolute tokens: warn at 60% and stop at 75% of the window (integer
  maths, rounded down). The line adds `(hand off soon)` past the first and `(hand off now)`
  past the second.

The reading goes to `context-usage.json` in the anneal home (`~/.anneal`, or `$ANNEAL_HOME`):
per session a SHA-256 of its id, `used_tokens`, `window` and `window_source`
(`env`, `model` or `default`), newest last, at most 64 sessions. Like a hook it fails open: on
any failure it still prints a line (`(input unreadable)`, `(usage not saved)`), writes one
`ERROR` line on stderr and exits 0; only a usage error exits 2.

## The context monitor

Hooks receive no usage field, so the monitor reads the session's reading from
`context-usage.json`. With none (no status line configured, or none yet this session) it is
silent. It keeps the level each session last reached (`context-monitor.json`, same format):

- usage rises past 60%: one warning, as `additionalContext`;
- usage rises past 75%: one stop, as `decision: "block"` with a reason naming `/handoff`.
  `PostToolBatch` then ends the agentic loop before the next model call; the user sees the
  reason, and the model sees it when the conversation continues;
- a jump past both fires the stop alone;
- usage falling below a threshold (a compaction, a `/clear`) re-arms it.

The new level is saved before anything fires, so a file that cannot be written never repeats
a crossing on every batch; its own failures are one `ERROR` line as the `systemMessage`, never a
block. It is declared under `PostToolBatch` rather than `PostToolUse *`, so it runs once per
batch of tool calls; given a `PostToolUse` input it answers as `PostToolUse`.

## `.continue-here`

Before every compaction the `handoff` hook writes `.continue-here` at the worktree root,
atomically. It adds `/.continue-here` to the repository's `<git-common-dir>/info/exclude` (once,
only when no line there names it yet), so the note is never an untracked change: `git status`,
`anneal mode` and the Stop hook do not see it, and `mode::scope` counts a root
`.continue-here` as scratch as well. The note:

```markdown
# Continue here

Written by anneal's handoff hook before an automatic compaction at <time>. ...

- Phase: build mode (plan `plans/p.md`, design tree `tree.json`)
- Task: `T2` (2 of 3): Write the output; 1 done
- Open gates: none
- Ledger (newest last): mode_set build; task_done task:T1
- Compact instructions: <what /compact was given>

## Notes

<kept from the previous file>
```

- **Phase**: the worktree's anneal mode (`mode::active`).
- **Task**: in build mode, the first task of the plan with no `task_done` in this worktree since
  the mode was entered, its position, and how many are done.
- **Open gates**: what the Stop hook would hold against stopping now: `[DEBUG-<id>]` leftovers
  and the playbook's missing events (its checks run in process, with no reply to judge).
- **Ledger**: the five newest events, `<type> <subject>` each.
- **Notes**: the `## Notes` section of the previous file and all after it, at most 8 KiB, is
  kept: that is where the agent writes what a script cannot know.

Each item is one line: control characters (CR and LF among them) and Unicode line separators in
a file name, task title, gate or error read as spaces, so no item can forge another line.

The hook records the SHA-256 of the note's head (everything before its `## Notes` line) for the
worktree in `<anneal home>/handoff-notes.json` (at most 64 worktrees). Notes are kept, and the
note is injected, only while the file's head is the one recorded: a hand-written or committed
`.continue-here`, or one whose head was edited, is neither. Edit only the notes section.

What it cannot read is listed under `Not read:`. Outside a git repository it writes nothing; it
never creates a ledger and never blocks compaction. Run by hand with `{}` on stdin (`anneal hook
handoff`), it writes the same note for a handoff by `/clear`.

Both the hook and the bootstrap read the note only as a regular file (a symlink is refused, not
followed) of at most 32 KiB, checking its length before reading it. The bootstrap injects a
recorded note at every session start while it exists, after the index, under a `## Handoff
note (.continue-here)` heading that frames it as data, not instructions. It runs the
injection scan (`docs/injection-scan.md`) over the note and, on a hit, adds one line naming
`.continue-here:<line>` and the kind of text, as the injection-scan hook does. An unrecorded
note is not injected; a one-line note says so. The skill deletes the file once the work is
picked up.

## The setup hint

While the anneal home has no `context-usage.json`, the bootstrap adds a one-line hint to set up
the status line, once per anneal home (it records `statusline-hint.json`). The first session
start can run before the status line's first run, so a user who has just set it up may still
see the hint once.
