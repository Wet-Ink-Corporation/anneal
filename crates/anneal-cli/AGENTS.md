# Crate charter: `anneal-cli`

## Purpose
The `anneal` binary: the deterministic layer behind the plugin's hooks and skills. It is the composition root, so argument parsing, output formatting and exit codes live here. Domain logic moves into library crates when a second consumer appears (ARC-07).

## Layer
binary. Owns process concerns: arguments, stdout and stderr, exit codes.

## Allowed dependencies
- Workspace crates: none yet.
- Third-party: none yet. Each addition is an H-16 decision recorded below.
- Never: anything that needs a network at build time.

## Public surface
A library target (`src/lib.rs`) holds the logic so doc tests run; `src/main.rs` stays a thin binary. The contract is the command line, the JSON on stdout and the documented exit codes. The library is workspace-internal, not published.

## Invariants
- `--help` and `--version` exit 0; a usage error exits 2 and writes nothing on stdout.
- Every error is one `ERROR <reason>` line on stderr.

## Errors
Usage errors exit 2. A write failure on stdout or stderr exits 1, since there is nowhere left to report it.

## Test strategy
Unit tests beside the code for parsing and output; integration tests in `tests/it/` run the built binary. Run with `cargo test -p anneal-cli`.

## Do not
- Do not print with `println!`; write through a locked handle.

## Decisions
| Date | Decision | Rule | Decided by |
|---|---|---|---|
