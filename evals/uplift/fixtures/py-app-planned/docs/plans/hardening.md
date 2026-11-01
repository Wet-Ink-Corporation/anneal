# Plan: hardening

- Spec: [hardening](../specs/hardening.md)

## Goal

`app` has help and version flags, a configurable cap, a retried delete, a report that never overwrites a file, an audit log that reports its own failures, and a duration parser.

## Architecture

Each task is a small change to one module under `src/` with its own `unittest` test file under `tests/`. The tasks are independent.

## Tasks

### T1: --help and --version exit 0

- Files: `src/app/cli.py`, `tests/test_flags.py`
- Test: `python3 -m unittest tests.test_flags`
- Blocked by: none

### T2: A configurable finding cap

- Files: `src/app/findings.py`, `tests/test_findings.py`
- Test: `python3 -m unittest tests.test_findings`
- Blocked by: none

### T3: delete retries a permission error

- Files: `src/app/cli.py`, `tests/test_delete_retry.py`
- Test: `python3 -m unittest tests.test_delete_retry`
- Blocked by: none

### T4: report --out never overwrites

- Files: `src/app/cli.py`, `tests/test_report_out.py`
- Test: `python3 -m unittest tests.test_report_out`
- Blocked by: none

### T5: The audit log reports its failures

- Files: `src/app/audit.py`, `tests/test_audit.py`
- Test: `python3 -m unittest tests.test_audit`
- Blocked by: none

### T6: parse_duration reads minutes and hours

- Files: `src/duration.py`, `tests/test_duration.py`
- Test: `python3 -m unittest tests.test_duration`
- Blocked by: none

## Review focus

- `src/app/cli.py`: `delete` must still refuse when the audit log cannot be written (T3, T5).
- `tests/`: each boundary and failure branch has its own test, not only the happy path.
