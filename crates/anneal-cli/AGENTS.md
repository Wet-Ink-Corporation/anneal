# Crate charter: `anneal-cli`

## Purpose
The `anneal` binary: the deterministic layer behind the plugin's hooks and skills. It is the composition root, so argument parsing, output formatting and exit codes live here. Domain logic moves into library crates when a second consumer appears (ARC-07).

## Layer
binary. Owns process concerns: arguments, stdout and stderr, exit codes. The code lives in an internal library target (`anneal_cli`, `src/lib.rs`) so tests and doctests can reach it; `src/main.rs` only passes the process's arguments and streams to `cli::run`.

## Allowed dependencies
- Workspace crates: `anneal-ledger` (event registry and store).
- Third-party: `clap` (derive; command tree and help), `serde` and `serde_json` (envelope, snapshots), `thiserror` (error enums), `sha2` (content hashes for the lint snapshot); dev: `tempfile`. Each addition is an H-16 decision recorded below.
- Never: anything that needs a network at build time.

## Public surface
The contract is the command line, the JSON envelope on stdout and the exit codes in `docs/exit-codes.md`. The `anneal_cli` library is internal (`publish = false`) and not a stable API.

Modules: `cli` (clap tree, `run`), `command` (closed `CommandName` set, proves / does-not-prove), `exit_codes` (the one exit-code table; `exit_codes::render` turns it into help and markdown), `error` (`CliError` and its global codes), `envelope` (`Envelope`, `Finding`), `hook` (`anneal hook <name>`, the fail-open commands `plugin/hooks/hooks.json` runs: `hook::bootstrap` is the SessionStart index (`hook::bootstrap::index`, generated into `docs/bootstrap-index.md`) plus the live state from the ledger; `hook::guard` is the fail-closed PreToolUse Bash guard: `guard::shell` lexes the command, `guard::denylist` (with `wrappers`, `git`, `rm`, `config`) denies or leaves it undecided, and `guard::Fallback` is the extension point for Clef D1, #30; see `docs/guard.md`; and the one-`ERROR`-line output hooks share; see `docs/hooks.md`), `baseline` (finding snapshots), `ledger` (`anneal ledger append|query|tail|journal`), `paths` (ledger store resolution, git common dir, `~/.anneal`), `lint` (`lint skills`: the 21 skill design rules, one checker per rule in `lint::skills::checker`, documented in `docs/skill-rules.md`; the shared `LintFinding` model and `lint::report`; `lint::skills::listing` lists skills for `eval`), `eval` (`eval routing`: Tier 1 lexical routing; `eval::routing` reads fixtures with `fixture`, scores with `score`, judges with `judge` and tabulates with `confusion`; schema and bar in `evals/README.md`).

To build out a stub: replace its `Stub` in `cli::Command` with a real `clap::Args` type, give it its own arm in `cli::dispatch` that writes an `Envelope` and returns `emit`'s code, and declare any new code in `exit_codes` (then regenerate the doc with `ANNEAL_BLESS=1 cargo test -p anneal-cli exit_codes_doc`).

## Invariants
- `--help` and `--version` exit 0 (only as the last argument); a usage error exits 2 and writes nothing on stdout.
- Every error is exactly one `ERROR <reason>` line on stderr, and nothing on stdout.
- No failure exits 0; the envelope's `ok` is true exactly when the exit code is 0. The one exception is `anneal hook <name>`, which fails open: it notes its failures in its output (or in one `ERROR` line on stderr when stdout cannot be written) and exits 0; only its usage errors exit 2 (`a_closed_stdout_still_exits_0_with_one_error_line`). A guard (`hook guard`, or a hook passed `--fail-closed`) fails closed instead: it denies what it cannot judge (bad or oversized input, an unparseable command) with a deny decision on stdout and exit 0, and exits 2 with one `ERROR` line when that decision cannot be written (`a_closed_stdout_exits_2_with_one_error_line`).
- The guard is silent when it allows; a denial is one `PreToolUse` deny decision whose reason is one `ERROR` line, plus a `gate_blocked` event, and a ledger failure never turns a deny into an allow (`tests/it/hook_guard.rs`, `hook::guard::tests`). The denylist never allows: it denies or leaves a command to `guard::Fallback`.
- A `hooks.json` entry sets `onFailure: "block"` exactly when it passes `--fail-closed` (`an_entry_blocks_on_failure_exactly_when_it_passes_fail_closed`).
- Every exit code is declared in `exit_codes`; `docs/exit-codes.md` is generated from it and a test fails on drift.
- Every `hooks.json` entry is exec form (`${CLAUDE_PLUGIN_ROOT}/libexec/anneal` plus `args`) with `onFailure` and an explicit `timeout`, runs an `anneal hook` subcommand the binary accepts, and has exactly one row in `hooks.metadata.json` (and every row an entry); the metadata validates against a strict DTO that agrees with `hooks.metadata.schema.json` (`tests/it/hooks_manifest.rs`).
- The bootstrap index is 200 words or fewer, lists `ENTRY_SKILLS` in order, and passes `anneal lint skills --index docs/bootstrap-index.md`; live state comes after it, only when it changed for the session, and the bootstrap never creates a ledger (`tests/it/hook_bootstrap.rs`).

## Errors
Usage errors exit 2, stubs 64, internal errors (git, snapshots, the ledger store) 70, and a write failure on stdout or stderr 74. A command's own outcomes (cycle, red test, findings) are envelopes with per-command codes, not errors.

## Test strategy
Unit tests beside the code for parsing and output; integration tests in `tests/it/` run the built binary. Run with `cargo test -p anneal-cli`. `tests/it/repo_gates.rs` runs the repository's own `plugin/skills` and `evals/routing` against the Tier 0 and Tier 1 bars, so CI's `cargo test` enforces them.

## Do not
- Do not print with `println!`; write through a locked handle.
- Do not open the ledger anywhere but through `anneal_ledger::Store`, and do not resolve its path anywhere but `paths::ledger_store` (or `paths::ledger_store_within`, its variant with a caller's git timeout).

## Decisions
| Date | Decision | Rule | Decided by |
|---|---|---|---|
| 2026-10-11 | Add `clap` (derive) for the command tree | H-16 | story #10 |
| 2026-10-11 | Add `serde`, `serde_json`, `thiserror`, dev `tempfile` (pre-approved) | H-16 | `.claude/temper.json` |
| 2026-10-11 | Internal library target plus thin binary, so doctests and tests reach the code | ARC-07 | coordinator (#10) |
| 2026-10-11 | Usage errors keep clap's 2 (a mis-invoked guard blocks); globals 64 not implemented, 70 internal, 74 output | OUT-06 | weigh-in wi-2415fc (default, two-way) |
| 2026-10-11 | Add `sha2` (no default features) for `lint skills --cache` content hashes, as weigh-in does | H-16 | story #12 |
| 2026-10-11 | `lint` exits 1 only on error findings; warnings and review items are listed and exit 0 | OUT-06 | story #12 (recommended option) |
| 2026-10-11 | Entry skills are a constant (`ENTRY_SKILLS`) documented in `docs/skill-rules.md`, not read from `hooks.metadata.json` (#13 owns that schema) | ARC-05 | weigh-in wi-7fad93 (default, two-way) |
| 2026-10-11 | Lint details (rule, checker, severity, fix) go in the envelope's `data.details`; `envelope::Finding` stays `{key, message}` | API-04 | weigh-in wi-37e954 (default, two-way) |
| 2026-10-11 | Depend on the new `anneal-ledger` crate; store resolution (flag, `$ANNEAL_STORE`, git common dir, folder) stays in `paths` so the ledger crate knows nothing of git | ARC-02 | story #11 |
| 2026-10-11 | A ledger schema error is an envelope with exit 2 and one `ledger/schema` finding, also on read when a stored event does not decode; store failures are 70 | OUT-06 | implementer (#11), weigh-in wi-68eae3 (default, two-way) |
| 2026-10-11 | Routing fixtures are a restricted YAML subset read by hand (no YAML crate); near misses are grouped by owner (`near_misses: {owner: [prompts]}`) | H-16 | story #15 (no new dependency approved; implementer default, two-way) |
| 2026-10-11 | `eval routing` exits 2 ("invalid fixtures or skills") when a fixture is invalid or missing, apart from 1 for a routing miss | OUT-06 | implementer (#15, default, two-way) |
| 2026-10-11 | Tier 1 scores a skill's name, trigger (description before "Do NOT use for") and `when_to_use`, with a light stemmer; a tie is a miss | n/a (design) | implementer (#15, default, two-way) |
| 2026-10-11 | `anneal hook` is a top-level command (`CommandName::Hook`, exit table `hook bootstrap: 0`) that fails open, also on a failed stdout flush (`cli::OnFailure::FailOpen`) | OUT-06 | implementer (#13), two-way |
| 2026-10-11 | The bootstrap index is Rust (`hook::bootstrap::index`) with a generated, drift-tested copy in `docs/bootstrap-index.md` for `lint skills --index` | ARC-05 | implementer (#13), two-way |
| 2026-10-11 | What each session was shown is kept as hashes in `bootstrap-sessions.json` beside the ledger, at most 64 sessions, keyed by Claude Code's session id | H-13 | implementer (#13), weigh-in wi-80dac4 (default, two-way) |
| 2026-10-11 | After `/clear` the live state is re-injected: Claude Code gives the cleared conversation a new session id, and its context no longer holds the state | n/a (design) | weigh-in wi-6f8350 (default, two-way) |
| 2026-10-11 | The bootstrap bounds its live-state lookup (git 2 s, 0.5 s per ledger statement, about 7 s worst case) under the hook's 15 s timeout instead of raising the timeout: with the 10 s defaults a stuck writer could cost 10 s per statement, about 100 s | H-13 | implementer (#13 review round 1), two-way |
| 2026-10-11 | No Claude Code version warning: hooks receive no version (checked against 2.1.296), and `CLAUDE_CODE_VERSION`, where present, is set by the remote runner and was seen stale | OUT-01 | weigh-in wi-319ca0 (default, two-way) |
| 2026-10-11 | The guard denies with Claude Code's `PreToolUse` JSON decision (`permissionDecision: "deny"`, reason one `ERROR` line) and exit 0, checked against 2.1.296; exit 2 is kept for "the decision could not be written" (`cli::OnFailure::FailClosed`, `exit_codes::HOOK_BLOCKED`), and `--fail-closed` makes any hook fail closed | OUT-06 | coordinator default (#14), two-way |
| 2026-10-11 | The guard's shell lexer is hand-written (no new dependency); stdin capped at 1 MiB, lexer nesting at 32, `bash -c`/`eval` nesting at 8; anything it cannot read is denied | H-13, H-16 | coordinator default (#14), two-way |
| 2026-10-11 | The `hook` exit table covers every hook (scope empty) rather than listing `bootstrap\|guard`, so the help column keeps its width | OUT-06 | implementer (#14), two-way |
