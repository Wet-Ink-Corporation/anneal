# Plan: notes list --json

- Spec: [notes list --json](spec.md)

## Goal

People scripting their notes run `notes list --json` and get one JSON array they can pipe into `jq`.

## Architecture

The flag is one more `clap` argument on the `list` subcommand in `src/cli.rs`. `src/list.rs`
gains a `render_json` beside the table renderer, reusing the `serde_json` dependency the notes
file already uses; errors keep going through the existing `report` function.

## Tasks

### T1: JSON array of notes

- Files: `src/cli.rs`, `src/list.rs`, `tests/list_json.rs`
- Test: `cargo test --test list_json array`
- Blocked by: none

1. Write a failing test: two notes, `notes list --json` prints one array sorted by `id` with `id`, `title` and RFC 3339 `created` (AC-1, AC-2).
2. Run the test command; it fails because `--json` is an unknown argument.
3. Add the flag and `render_json`; run the test command; it passes.

### T2: Empty list

- Files: `src/list.rs`, `tests/list_json.rs`
- Test: `cargo test --test list_json empty`
- Blocked by: T1

1. Write a failing test: with no notes, `notes list --json` prints `[]` and exits 0 (AC-3).
2. Run the test command; it fails because the empty case prints the "no notes" line.
3. Return the empty array before the "no notes" branch; run it green.

### T3: Errors under --json

- Files: `src/list.rs`, `tests/list_json.rs`
- Test: `cargo test --test list_json unreadable`
- Blocked by: T1

1. Write a failing test: an unreadable notes file with `--json` prints one stderr line, nothing on stdout, exit 1 (AC-4).
2. Run the test command; it fails if `render_json` starts writing before the read fails.
3. Read every note before writing; run it green.

### T4: The table is unchanged

- Files: `tests/list_table.rs`
- Test: `cargo test --test list_table`
- Blocked by: none

1. Write a snapshot test of `notes list` without `--json` against today's table (AC-5).
2. Run the test command; it passes on today's code, which pins the table before T1 lands.

## Review focus

- `src/list.rs`: AC-4, nothing reaches stdout before every note is read (T3).
- `tests/list_json.rs`: the `created` format is RFC 3339 in UTC, not the local time zone (T1).
