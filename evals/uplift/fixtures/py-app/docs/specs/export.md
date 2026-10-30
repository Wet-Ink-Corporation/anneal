# Spec: export

Status: finished (requirements agreed 2026-10-10)

## Problem

The findings of a log file can only be read on the terminal today, so a user who wants them in a spreadsheet or another tool copies them by hand.

## Users

People who run `app report` and want its findings in a file that another tool can read.

## Requirements

- Export the findings of a log file as CSV or as JSON.
- Never overwrite an existing file unless the user asks for it.

## Acceptance criteria

1. WHEN the user runs `app export <file> --format csv`, THE tool SHALL write one CSV row per finding with the columns `line` and `text`.
2. WHEN `--format json` is given, THE tool SHALL write a JSON array of `{"line", "text"}` objects.
3. IF the output file already exists, THEN THE tool SHALL refuse with exit code 2 unless `--force` is given.

## Out of Scope

- Streaming output for files larger than memory.

## Open questions

- None.

## Constitution check

No constitution.
