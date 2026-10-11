# Crate charter: `anneal-cli`

## Purpose
The `anneal` binary: the deterministic layer behind the plugin's hooks and skills. It is the composition root, so argument parsing, output formatting and exit codes live here. Domain logic moves into library crates when a second consumer appears (ARC-07).

## Layer
binary. Owns process concerns: arguments, stdout and stderr, exit codes. The code lives in an internal library target (`anneal_cli`, `src/lib.rs`) so tests and doctests can reach it; `src/main.rs` only passes the process's arguments and streams to `cli::run`.

## Allowed dependencies
- Workspace crates: none yet.
- Third-party: `clap` (derive; command tree and help), `serde` and `serde_json` (envelope, snapshots), `thiserror` (error enums); dev: `tempfile`. Each addition is an H-16 decision recorded below.
- Never: anything that needs a network at build time.

## Public surface
The contract is the command line, the JSON envelope on stdout and the exit codes in `docs/exit-codes.md`. The `anneal_cli` library is internal (`publish = false`) and not a stable API.

Modules: `cli` (clap tree, `run`), `command` (closed `CommandName` set, proves / does-not-prove), `exit_codes` (the one exit-code table; `exit_codes::render` turns it into help and markdown), `error` (`CliError` and its global codes), `envelope` (`Envelope`, `Finding`), `hook` (one `ERROR` line), `baseline` (finding snapshots), `paths` (git common dir, `~/.anneal`).

To build out a stub: replace its `Stub` in `cli::Command` with a real `clap::Args` type, give it its own arm in `cli::dispatch` that writes an `Envelope` and returns `emit`'s code, and declare any new code in `exit_codes` (then regenerate the doc with `ANNEAL_BLESS=1 cargo test -p anneal-cli exit_codes_doc`).

## Invariants
- `--help` and `--version` exit 0 (only as the last argument); a usage error exits 2 and writes nothing on stdout.
- Every error is exactly one `ERROR <reason>` line on stderr, and nothing on stdout.
- No failure exits 0; the envelope's `ok` is true exactly when the exit code is 0.
- Every exit code is declared in `exit_codes`; `docs/exit-codes.md` is generated from it and a test fails on drift.
- There is no `anneal hook` command until #13; `hooks.json` must not call one (a usage error exits 2, which Claude Code reads as "block").

## Errors
Usage errors exit 2, stubs 64, internal errors (git, snapshots) 70, and a write failure on stdout or stderr 74. A command's own outcomes (cycle, red test, findings) are envelopes with per-command codes, not errors.

## Test strategy
Unit tests beside the code for parsing and output; integration tests in `tests/it/` run the built binary. Run with `cargo test -p anneal-cli`.

## Do not
- Do not print with `println!`; write through a locked handle.

## Decisions
| Date | Decision | Rule | Decided by |
|---|---|---|---|
| 2026-10-11 | Add `clap` (derive) for the command tree | H-16 | story #10 |
| 2026-10-11 | Add `serde`, `serde_json`, `thiserror`, dev `tempfile` (pre-approved) | H-16 | `.claude/temper.json` |
| 2026-10-11 | Internal library target plus thin binary, so doctests and tests reach the code | ARC-07 | coordinator (#10) |
| 2026-10-11 | Usage errors keep clap's 2 (a mis-invoked guard blocks); globals 64 not implemented, 70 internal, 74 output | OUT-06 | implementer (#10), open for review |
