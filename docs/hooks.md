# Hooks

anneal's hooks are declared in `plugin/hooks/hooks.json` and described, one row per hook, in
`plugin/hooks/hooks.metadata.json`. Each hook runs `anneal hook <id>` through the launcher.

## `hooks.json`

Every entry is **exec form**: `command` is the launcher by path and `args` are its arguments, so
no shell parses them. Every entry also declares `onFailure` (`continue` to fail open, `block` for
a guard) and an explicit `timeout` in seconds. `onFailure` needs Claude Code 2.1.295 or later;
older versions ignore it.

```json
{ "type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/libexec/anneal",
  "args": ["hook", "bootstrap"], "timeout": 15, "onFailure": "continue" }
```

## `hooks.metadata.json`

The sidecar records, for each hook, why it exists, so scaffolding can be re-tested and removed
as models improve. Its JSON Schema is `plugin/hooks/hooks.metadata.schema.json`.

| Field | Meaning |
|---|---|
| `id` | Stable id: the entry runs `anneal hook <id>`. Skills may name it as a MUST's enforcer (rule 10). |
| `event` | The Claude Code event the entry is declared under. |
| `matcher` | The entry's matcher, exactly (empty when it has none). |
| `fingerprint` | `sha256:` and the first 16 hex digits of SHA-256 over `<event>\n<matcher>\n<the hook object as compact JSON, keys sorted>`. It changes whenever the entry does, so the row is re-read. |
| `on_failure` | The entry's `onFailure`. |
| `role` | `scaffolding` (helps a model generation; re-test and remove later) or `guard` (enforces a gate). |
| `model_generation` | The model generation the hook was added for, such as `claude-5`. |
| `source` | Where the hook is specified: blueprint section and story. |

`cargo test -p anneal-cli hooks_manifest` checks both files: every entry is exec form with
`onFailure` and a timeout, every entry has exactly one row (same id, event, matcher, fingerprint
and failure policy) and every row an entry, every id is an `anneal hook` subcommand the binary
accepts, the metadata parses into a strict DTO (no unknown or missing field), and the schema's
required fields, properties, enums and patterns agree with that DTO. It also checks that an entry
has `onFailure: "block"` exactly when its arguments include `--fail-closed`. When a test reports
a new fingerprint after you change an entry, copy it into the row.

## Failing open, and guards failing closed

Hooks fail open. `anneal hook <id>` notes its failures in its output, or in one `ERROR` line on
stderr when stdout cannot be written, and exits 0. Only a usage error exits 2, which is why the
manifest test runs every entry's arguments through the binary.

Guards fail closed. A guard (`hook guard`, or any hook passed `--fail-closed`) denies what it
cannot judge. When its output cannot be written it exits 2 with one `ERROR` line, which Claude
Code reads as "block". Its entry passes `--fail-closed` and sets `onFailure: "block"`, so a crash
or a timeout blocks too.

With no binary at all, the launcher exits 0 for a hook (2 for one passed `--fail-closed`). For
`hook bootstrap` it also prints an install hint as `SessionStart` output, so the session starts
and says why anneal is off.

## `bootstrap` (SessionStart)

Matcher `startup|clear|compact|resume`, so it runs on every session start except a fork. It reads
Claude Code's `SessionStart` input on stdin (at most 64 KiB; it uses `session_id` and `cwd`) and
writes `{"hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": ...}}`.
The context is, in order:

1. **The index**, every time (stable first): what anneal is, the seven entry skills with their
   triggers, and the user-invoked commands, in 200 words or fewer. Its text is generated into
   `docs/bootstrap-index.md` (`ANNEAL_BLESS=1 cargo test -p anneal-cli bootstrap_index_doc`),
   and `anneal lint skills <skills> --index docs/bootstrap-index.md` accepts it. `lint skills`
   runs its index rule only when it lints at least one skill; `plugin/skills` has none yet, so
   the test (`lint_skills_accepts_the_committed_index`) lints a fixture skill.
2. **The live state**, only when it changed for this session (volatile last): the newest five
   ledger events. The ledger is found as `anneal ledger` finds it (`$ANNEAL_STORE`, else the git
   common dir, else `.anneal/` in the folder). With no ledger file there is no live state, and
   the bootstrap never creates one. A ledger that cannot be read gives a one-line reason instead.
3. **Notes**, one line each, for anything that failed (unreadable input, no working directory,
   the sessions file).

An empty (zero-length) ledger file counts as no ledger: opening it would give it the schema.

**Time budget.** The entry's `timeout` is 15 s, and when it runs out Claude Code drops the whole
output, index included. So the live-state lookup is bounded well under it: git may take 2 s to
find the ledger (`anneal ledger` allows 10 s), and each ledger statement waits at most 0.5 s for
another process's lock (the store's default is 10 s per statement). Opening the ledger and
reading its tail run about ten statements, so even a stuck writer costs about 5 s, and the worst
case is about 7 s plus reading stdin and writing the sessions file. A lookup that runs out of
time becomes the one-line "could not be read" note, and the index still arrives.

What each session was shown is kept beside the ledger in `bootstrap-sessions.json`: a hash of
the session id and a hash of the live state, for the 64 most recent sessions. The same session
with an unchanged ledger gets the index alone. Claude Code gives a cleared conversation a new
session id, so after `/clear` the live state is injected again into the now empty context; a
compaction or resume keeps the id. Without a session id, the live state is injected every time.

Not yet in the live state: the phase and standing orders (nothing records them yet), and open
gates (the ledger records `gate_blocked` but not a gate reopening; blocked gates show in the
tail). Claude Code does not pass its version to hooks, so the bootstrap cannot warn about
versions older than 2.1.295.

## `guard` (PreToolUse)

Matcher `Bash`, `timeout` 5 s, `onFailure: "block"`, arguments `hook guard --fail-closed`. It
reads Claude Code's `PreToolUse` input on stdin (at most 1 MiB; it uses `tool_name`,
`tool_input.command`, `session_id` and `cwd`), lexes the command the way a shell does, and
checks it against a deterministic denylist: force pushes to protected branches, `rm -rf` outside
the temporary and scratch directories, `git add -f`, and Bash writes to lint or hook config.

An allowed command gets no output. A denied one gets
`{"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny",
"permissionDecisionReason": "ERROR guard <rule>: <reason>"}}` and a `gate_blocked` event in the
ledger (`point` is `guard.<rule>`), recorded within the hook's budget (git 1 s, 500 ms per
ledger statement). Unreadable input, and commands it cannot read or model, are denied. What
the denylist does not decide is asked of Clef D1 (`noul destructive_or_irreversible`, #30) in
process, its command redacted: in shadow mode (the default, and the only mode in effect until the
HTTP client lands: `NoHttpClient` makes every call unreachable, and unanswered calls do not end
the shadow period) the would-be action is
logged and the verdict is unchanged; in enforce mode 0.90 or more denies (rule `destructive`),
0.40 or more asks (`"permissionDecision": "ask"`), less defers, and no answer asks; D1 set to
enforce that cannot run asks, and an answer that cannot be logged never becomes an allow. With
Clef off nothing is asked. The guard measures its 4.5 s budget from its start, and a deny with too
little time left skips the ledger record, with a note. A denylist denial never reaches Clef. The denylist, the bypass handling and
D1 are documented in `docs/guard.md`.

## `preconditions` (PreToolUse)

Matcher `Edit|Write`, `timeout` 5 s, `onFailure: "block"`, arguments
`hook preconditions --fail-closed`. It reads Claude Code's `PreToolUse` input on stdin (at most
1 MiB; it uses `tool_name`, `tool_input.file_path`, `session_id` and `cwd`) and the worktree's
anneal mode (`anneal mode build|diagnose|off`, a `mode_set` event in the ledger). The worktree
is the edited file's own, found from its longest existing ancestor directory, not the hook's
cwd. For a file in no git repository, with no ledger, no mode or mode `off` it allows silently and never creates a ledger. In build mode it
denies source edits until the plan's spec passes `lint spec`, the design tree is confirmed at its
current hash, the tree was clean at entry and the plan is approved and current; in diagnose mode
until `anneal repro record` records a `repro_red` for the worktree after entry. Test files, the plan and tree, scratch
directories and prose documents are always allowed; the plan's `Spec:` line exempts nothing. A denial is the
same deny decision as the guard's, with one `ERROR` line naming the missing precondition and the
command that satisfies it, plus a `gate_blocked` event (`point` is `preconditions.<check>`).
Unreadable input, and a ledger or git it cannot read, are denied. See `docs/preconditions.md`.

## `fast-check` (PostToolBatch)

No matcher (`PostToolBatch` takes none), `timeout` 15 s, `onFailure: "continue"`, arguments
`hook fast-check`. Claude Code 2.1.296 fires `PostToolBatch` once after every call of a batch of
parallel tool calls has resolved, before the next model request, with `tool_calls` (each
`{tool_name, tool_input, tool_use_id, tool_response}`); `PostToolUse`, by contrast, fires once per
call. So ten parallel edits run these checks once. It reads at most 8 MiB of stdin (it uses
`cwd` and `tool_calls`).

It runs only when the batch holds a file edit (`Edit`, `Write` or `MultiEdit` `file_path`,
`NotebookEdit` `notebook_path`) and the session's worktree is in an anneal mode (`anneal mode
build|diagnose`); outside a repository, with no ledger or no mode it is silent and never creates
a ledger. The changed files are the edited files inside the worktree, each once, at most 128;
an input that lists no calls (or cannot be read, which is noted) takes them from `git status`
(changed and untracked, 2 s). Then, within 13 s in total:

| Order | Check | Limit | A failure is |
|---|---|---|---|
| 1 | `anneal floor-guard` | 6 s | exit 1: a **block**; anything else (exit 2, a timeout): a note |
| 2 | `check_fast` from `.anneal/config.json` (an argv array, no shell), when set | 5 s | a note with its exit and the last 2 KiB of its output |
| 3 | `anneal lint terms --glossary CONTEXT.md` on the changed files, when `CONTEXT.md` exists | 2 s | a note with the hits |
| 4 | `anneal lint spec` or `anneal lint plan` for each changed `.md` file whose nearest `specs` or `plans` directory names it | 2 s each | a note with the findings |

When every check passes it writes nothing. A block is
`{"decision": "block", "reason": "ERROR fast-check: anneal floor-guard reports a lowered floor: ..."}`,
one line, which Claude Code hands back to the model. Notes go in
`{"hookSpecificOutput": {"hookEventName": "PostToolBatch", "additionalContext": ...}}`. Its own
failures (the ledger, git, a check that cannot start) are notes too: the hook never blocks on
them. `check_fast` writes to a fresh temporary file rather than a pipe, so a process it leaves
running cannot hold the hook open past its timeout; the file is removed afterwards.

Limits:

- On a timeout only the child itself is killed. A process it started (a `git` call under a
  killed `anneal floor-guard`, anything `check_fast` starts) can outlive it and keep running;
  std has no portable process-group kill (as for `anneal task done`, `docs/task.md`).
- On Windows a child's handles are inherited by the processes it starts. A killed
  `anneal floor-guard` can leave a `git` call holding its output pipe, and the hook waits for
  that git to exit on its own (nothing bounds it once floor-guard is killed), so it can run
  past its 15 s timeout;
  Claude Code then drops its output and carries on (it fails open). A process `check_fast`
  leaves running keeps the log file open, so removing it fails: that is a note, and the file
  stays in the temporary directory. On unix the log is removed and the hook returns as soon as
  `check_fast` itself exits (`a_process_check_fast_leaves_behind_does_not_hold_the_hook`).

## `completeness` (Stop)

No matcher, `timeout` 30 s, `onFailure: "continue"`, arguments `hook completeness`. It reads
Claude Code's `Stop` input on stdin (at most 4 MiB; it uses `session_id`, `transcript_path`,
`cwd`, `stop_hook_active` and `last_assistant_message`, checked against Claude Code 2.1.296's
schema; without `last_assistant_message`, the last assistant message in the transcript's final
1 MiB) and checks the git worktree of `cwd`. Outside a repository it does nothing. Only the
final assistant message is judged: a claim made in an earlier message of the same turn, before
more tool calls, is not seen.

**Hard gates** block with `{"decision": "block", "reason": "anneal completeness: ..."}`:

| Gate | Blocks when | Point |
|---|---|---|
| claim | the reply claims success and a source file changed after the worktree's latest `test_run` started, or that run is not green with exit 0, or has no `duration_ms`, or there is none | `completeness.claim` |
| leftovers | a `[DEBUG-<id>]` tag is left in a file git knows of (`repro::debug_leftovers`, the scan `anneal repro leftovers` runs) | `completeness.leftovers` |

A file the leftover scan could not read (over 1 MiB, past the 64 MiB budget, or a name it cannot
open) is a nudge naming how many were skipped and pointing at `anneal repro leftovers`.

- **Success words** (`hook::completeness::claim::CLAIMS`): `all tests pass`, `tests pass`
  (`test` or `tests`, and `pass`, `passes`, `passed` or `passing`), `works now`, `all green`,
  `done`, `passes`, `passed`, `passing`, `green`, `fixed`, `complete`, `completed`, `finished`,
  `succeeded` and `succeeds`, as whole words in one clause, in any letter case. Fenced code
  blocks, code spans and HTML comments are quoted text, not claims, and a claim with `not`,
  `never`, `no` or a word ending in `n't` one or two words before it in the same clause is not
  one. Clauses end at `.`, `!`, `?`, `;`, `:`, `,`, a dash and a line end, so "No failures.
  Done." claims. Some false positives ("done reading the logs") are accepted: the block cap
  bounds them.
- **Source edits** are the files `git status --porcelain -z --untracked-files=all` lists
  (tracked changes and untracked files git does not ignore) that the preconditions hook counts
  as source (not tests, prose documents or scratch paths). Their latest modification time, in
  milliseconds, must be earlier than the start of the latest `test_run` tagged with the worktree
  (`anneal task done` tags it): its `ts` minus its `duration_ms`. `task done` stamps `ts` after
  the command ends, truncated to the second, so the computed start is never later than the real
  one and up to a second earlier: an edit made while the tests ran always counts as untested,
  and one made in the second before they started may too (a block, never a pass). Runs stamped
  more than 60 s in the future are passed over (of the latest 16). There is no ledger edit event
  to use instead. A claim with no changed source file needs nothing. Not seen: changes already
  committed (git no longer lists them) and deleted files (they have no time). Not checked: what
  the run's command was (any command `task done` ran green counts, `git --version` included),
  and whether the run was recorded by `task done` at all: `anneal ledger append` can write a
  green `test_run` by hand. The gate keeps an honest agent honest; it is not a defence against
  one that forges evidence.

**Nudges** are context only (`hookSpecificOutput.additionalContext`), from the active playbook's
rule table (`hook::completeness::playbook::RULES`, one row per requirement, so #42 adds rows):

| Mode | Requires, after the mode was entered, tagged with the worktree | When |
|---|---|---|
| build | a green `test_run` (`anneal task done`) | a source file changed |
| diagnose | a `repro_red` (`anneal repro record`) | always |
| none or `off` | nothing | |

**Block cap.** It blocks at most 2 times in a row per session, stricter than Claude Code's own
cap of 8 (`CLAUDE_CODE_STOP_HOOK_BLOCK_CAP`). The count is kept in the anneal home directory
(`$ANNEAL_HOME`, else `~/.anneal`) in `stop-blocks.json`: per session a hash of its id, its
blocks in a row and the transcript's length at the last block, at most 64 sessions. A stop with
no hard gate resets it, and so does a tool call: the transcript bytes written since the last
block holding `"type":"tool_use"` (a transcript that shrank, or grew by more than 16 MiB, counts
as one). Past the cap, and whenever the count cannot be read or saved, the gates become context
saying why it did not block; a `stop-blocks.json` that is not a regular file (a directory,
symlink or FIFO) is refused before it is opened. When `stop_hook_active` is true (the stop follows a block) it
writes nothing and counts nothing, as Claude Code asks.

Each block appends one `stop_blocked` per gate to the ledger when one exists; it never creates
one. Anything it cannot check (git, the ledger, the counter, unreadable input) is one `ERROR`
line as `systemMessage`, and the stop goes on.

## `status` (SubagentStop)

No matcher, `timeout` 15 s, `onFailure: "continue"`, arguments `hook status`. It reads Claude
Code's `SubagentStop` input (at most 4 MiB; it uses `session_id`, `cwd`, `agent_id`,
`agent_type`, `agent_transcript_path` and `last_assistant_message`) and appends one
`subagent_status` to the ledger when one exists (it never creates one). It never blocks and
writes nothing unless something failed (one `ERROR` line as `systemMessage`).

- **Status block** (`hook::status::block`, values and sections as data): the last line whose
  text starts `Status:` (list markers, quote markers, `*` and code-span backticks ignored; code
  fences do not hide it, since temper's template is fenced), with `DONE`, `DONE_WITH_CONCERNS`,
  `NEEDS_CONTEXT` or `BLOCKED`, then a line starting each of `Gate:`, `Review:`, `Changed:`,
  `Decisions needed:` and `Not verified:` after it. No status line, an unknown value (temper's
  `WIP` included) or a missing section is the event's `violation`, with the sections in
  `missing`; the status, when known, is still recorded.
- **Timing and tokens.** The input has no timing or token fields (checked against Claude Code
  2.1.296), so they come from the subagent's transcript, read whole up to 32 MiB: `duration_ms`
  from its first to its last `timestamp`, and `tokens` summed over its assistant messages, each
  message's `usage` counted once although every content-block line repeats it. A longer
  transcript is recorded without them.

## `route` and `ruled-before` (UserPromptSubmit)

No matcher (`UserPromptSubmit` takes none), `timeout` 5 s each, `onFailure: "continue"`,
arguments `hook route` and `hook ruled-before`. Both read Claude Code's `UserPromptSubmit`
input (at most 4 MiB; they use `session_id`, `transcript_path`, `cwd` and `prompt`) and ask
Clef one decision point each, in process, through `decide`'s own code (`decide::in_process`
and `client::ask_recording`, as the guard's D1 does): the same configuration
(`docs/decide.md`), thresholds file, decision log, modes and 50-decision shadow period as
`anneal decide`, without starting a second process.

Shared behaviour:

- **Clef off** (no account, or the point's mode `off`): the hook does nothing. It writes
  nothing, reads no file and creates nothing in the anneal home.
- **Shadow** (the default, and every point's first 50 answered decisions even when `enforce`
  is configured): the question is asked and logged, and nothing is injected.
- **Enforce**: at most one line of `additionalContext` per hook per prompt, and only when the
  point's rule gives its acting action (`hint`, `surface`). The fixed wording comes first and
  the volatile part (skill, ruling) last.
- **No answer** (unreachable, timed out, bad response): the row is logged and the hook is
  silent. The binary has no HTTP client yet, so today every call is unreachable. Each call is
  bounded by `decide`'s 2 s timeout.
- **Not logged**: an answer whose row (or answered count) could not be written is never acted
  on, in enforce mode too: nothing is injected, route keeps nothing to label, and the failure is
  the hook's `ERROR` line. Unlike the guard, a prompt hook has no safe action to keep, so silence
  is the fail-open choice.
- **Time**: each hook measures a 4.5 s budget from its start on the monotonic clock (the
  `hooks.json` timeout is 5 s). Clef is asked only while its whole 2 s timeout fits in what is
  left; otherwise nothing is asked and the hook writes `ERROR <hook>: Clef was not asked: ...`.
  `ruled-before` gives git at most 1 s to find the repository, and never the time Clef needs.
- **The prompt sent** (as `{"input": <prompt>}` in the compact state) is whitespace-collapsed,
  redacted, and cut to 1,024 characters (the compact state's own cap) with `…` marking a cut,
  by `decide::state::cut_chars` as for the guard's command. Redaction (`decide::redact`)
  applies the guard's word rules (listed in `docs/guard.md`) to each word, even behind
  punctuation such as a quote or bracket: an environment-style or credential-named assignment
  (its name's own quotes stripped, as in `export "API_KEY"=x`), a credential-named key
  anywhere in a word (`--from-literal=password=x`, `{"password":"x"}`, `/:_authToken=x`), a
  credential-named option or configuration key (`--password x`, `-storepass x`,
  `user.password x`; not `--password-stdin`), the secret options of the known tools named
  earlier in the prompt (`mysql -p<redacted>`, `docker login -p <redacted>`, quotes around the
  option removed) and the word after `vault login`, the word after `Bearer` (and after `Basic` unless it is
  plain lower-case letters), the userinfo of a URL, and obvious tokens (`sk-`, `ghp_`, `gho_`,
  `ghu_`, `ghs_`, `ghr_`, `github_pat_`, `xox[abps]-`, `glpat-`, `hvs.`, `hvb.` and `eyJ` prefixes, AWS access
  key ids, and opaque words of 32 or more letters and digits mixed). Since prose has no command
  end, what one word says about the next is carried across words: a header name
  (`Authorization:`, `X-Api-Key:`, `Cookie:`, or one with its value attached, as in
  `Cookie:a=1;`) hides the words through its quote when it opens one (`"Cookie: s=1; t=2"`),
  else its next two words (scheme and credential); a quote
  opened before a name, in a value or by a hidden word is redacted through the word that
  closes it, or to the end of the prompt when none does (`DB_PASS='p w' then go` becomes
  `DB_PASS=<redacted> then go`, `TOKEN=abc"def ghi" then go` becomes
  `TOKEN=<redacted> then go`, `curl -u "me:my pass" x` becomes `curl -u "<redacted> x`). The
  guard's comment and heredoc rules do not apply. This is a filter, not a secret scanner. The
  rows keep no `input`, only the state's hash.
- **Failures** that are not Clef's (a malformed `clef_mode`, account or token, an unreadable
  thresholds file or log, an answer that could not be logged, an invalid ruling, unreadable
  input, a spent budget) are one `ERROR <hook>: ...` line as `systemMessage`, and the prompt
  goes on.

### `route` (D3)

It asks two questions at once, the size on a second, scoped thread, so together they take at
most one 2 s timeout:

- the **phase**, a `choice` over the phases of `plugin/playbooks/route.json` (built into the
  binary), each with its entry skill:

  | Phase | Entry skill |
  |---|---|
  | `define` | `grill` |
  | `specify` | `spec` |
  | `plan` | `plan` |
  | `build` | `build` |
  | `diagnose` | `diagnose` |
  | `verify` | `verify` |
  | `ship` | none until the `ship` skill lands |
  | `answer` | none: a question or chat that needs no workflow |

  A test keeps every entry skill a shipped `plugin/skills/<name>/SKILL.md` among the bootstrap
  index's entry skills, and every shipped entry skill the entry of exactly one phase.
- the **size**, a `score` read as the nearest of four levels, each with a depth: `trivial`
  (direct), `small` (light), `medium` (standard), `large` (full). The size's rule only
  records (`record`); it never acts on its own.

When the phase's probability reaches 0.60 (`hint`), the point is in enforce mode and the phase
has an entry skill, it injects:

```text
anneal route: load the `build` skill before acting (phase build, size medium, depth standard).
```

Without a size answer the size and depth are left out.

**Outcome label.** Each answered phase decision is remembered in its session's own file,
`route-pending/<sha256 of the session id>.json` beside the decision log (the decision id, its
top phase and the transcript's length; at most 64 files, the oldest dropped). Sessions never
share a file, so parallel sessions lose nothing, and the next prompt claims the file by an
atomic rename before labelling, so a decision is labelled at most once. At the session's next prompt the
hook reads what the transcript gained since then (at most 16 MiB) and labels the decision with
`decide label`'s row: the top phase when its entry skill was invoked through the `Skill` tool
(`build` or `anneal:build`), else the phase of the first entry skill invoked, else, with no
entry skill invoked, the top phase when it has no skill and `answer` otherwise. So
`anneal calibrate` scores the label as positive exactly when the hinted skill was invoked. Not
labelled: a session's last decision, one whose transcript was rewritten in between (it is
shorter, or no longer ends a line where it ended) or grew past the cap, and one whose file was
dropped as the oldest. A slash command the human types is not a `Skill` tool call and does not count.

### `ruled-before` (D6)

It gathers the rulings of the repository around `cwd` (its git worktree root, found within
1 s or what the budget can spare beside Clef's timeout, whichever is less, or `cwd` outside
one):

1. every ruling in `.out-of-scope/` (`rulings::list_within`, `docs/define.md`), by id, from
   at most 4,096 directory entries and 256 ruling files, regular files only (a symlink is not
   followed). An invalid ruling, or a store over either bound, fails the hook with an `ERROR`
   line naming the file or directory, and nothing is asked;
2. then the accepted weigh-in decision atoms in `.kb/_intake/decisions/*.md`, read from their
   frontmatter alone: `weighin_item` (the `wi-` id, the option), `status` (only `accepted`
   counts) and `title`, else `question`, as the summary. At most 4,096 directory entries are
   looked at and 256 atoms read, 16 KiB of each. An atom that does not read or parse is
   skipped, as is one a ruling already cites as its `weigh_in_ref`. No weigh-in binary is
   needed; `weighin open --all --json` as a third source arrives with #37.

An id must be a decide option (1 to 64 of `A-Z a-z 0-9 _ . : -`) and not `none`. At most 31
rulings are offered, rulings first, then atoms, each in id order, with `none` last; summaries
are cut to 120 bytes. With no rulings nothing is asked. The state is
`{"input": <prompt>, "rulings": [{"id", "summary"}, ...]}`.

When the top option is a ruling with probability 0.70 or more (`surface`) in enforce mode, it
injects:

```text
anneal ruled-before: this was ruled on already; do not re-open it unless the human asks to: wi-a53271 (Which licence should anneal ship under: MIT OR Apache-2.0).
```

**Outcome label: not yet.** D6's outcome is a human override ("yes, re-open it"), and nothing
in this story observes one deterministically. The design for the follow-up: when the hook
surfaces a ruling it remembers the decision per session, as `route` does; at the next prompt a
ruling-specific confirmation (an `AskUserQuestion` answer, or a `/anneal:reopen <id>` command
that records `ruling_reopened` in the ledger) labels it `none` (overridden), and any other next
prompt labels it with the ruling's id (it held). Until then D6 is labelled only by hand with
`anneal decide label`.
