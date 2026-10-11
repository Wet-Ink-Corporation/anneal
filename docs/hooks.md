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
required fields, properties, enums and patterns agree with that DTO. When a test reports a new
fingerprint after you change an entry, copy it into the row.

## Failing open

Hooks fail open. `anneal hook <id>` notes its failures in its output, or in one `ERROR` line on
stderr when stdout cannot be written, and exits 0. Only a usage error exits 2, which is why the
manifest test runs every entry's arguments through the binary.

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
