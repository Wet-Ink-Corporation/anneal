# Spec: hardening

- Tree: [hardening](../design/hardening.json)

## Problem

`app` lacks the flags every command line has, its finding cap is fixed, a delete that hits a briefly locked file fails at once, a report can only go to the terminal, the audit log hides its own failures, and nothing reads a duration such as `5m`.

## Users

People who run `app` by hand and from scripts.

## Requirements

- `-h`/`--help` and `-V`/`--version` work and exit 0.
- The finding cap is a parameter of `collect`, 1,000 by default.
- `delete` retries a permission error twice before giving up.
- `report --out <file>` writes the report to a file and never overwrites one.
- The audit log reports a failed write or close instead of hiding it.
- `parse_duration` in `src/duration.py` reads a duration in minutes or hours as seconds.

## Acceptance criteria

- **AC-1**: WHEN the user runs `app --help` or `app -h`, THE tool SHALL print the usage and exit 0.
- **AC-2**: WHEN the user runs `app --version` or `app -V`, THE tool SHALL print `app 0.1.0` and exit 0.
- **AC-3**: WHEN `collect` is given more matching lines than its `limit`, THE tool SHALL keep the first `limit` findings and count the rest in `omitted`.
- **AC-4**: IF removing the file raises a permission error three times, 100 ms apart, THEN THE tool SHALL print the error and exit 1.
- **AC-5**: IF the `--out` file already exists, THEN THE tool SHALL exit 2 and leave the file unchanged.
- **AC-6**: IF the audit line cannot be written or the log file cannot be closed, THEN THE tool SHALL return False from `audit.record` without raising.
- **AC-7**: WHEN `parse_duration` is given `5m` or `2h`, THE parser SHALL return 300 or 7200; IF it is given any other text, THEN THE parser SHALL raise `ValueError`.

## Out of Scope

- Help text for each subcommand.
- Retrying errors other than a permission error.
- Durations in other units, or combined (`1h30m`).

## Open questions

- None.

## Constitution check

No constitution.
