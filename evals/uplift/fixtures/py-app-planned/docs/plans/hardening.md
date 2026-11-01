# Plan: hardening

- Spec: [hardening](../specs/hardening.md)

## Goal

`app` has help and version flags, a configurable cap, a retried delete, a report that never overwrites a file, and an audit log that reports its own failures.

## Architecture

Each task is a small change to one module under `src/app/` with its own `unittest` test file under `tests/`. The tasks are independent.

## Tasks

### T1: --help and --version exit 0

- Files: `src/app/cli.py`, `tests/test_flags.py`
- Test: `python3 -m unittest tests.test_flags`
- Blocked by: none

1. Write the failing test for AC-1 and AC-2.
2. Run the test command; it fails because `main` prints the usage and returns 2.
3. Handle the flags in `main`; run the test command; it passes.

### T2: A configurable finding cap

- Files: `src/app/findings.py`, `tests/test_findings.py`
- Test: `python3 -m unittest tests.test_findings`
- Blocked by: none

1. Write the failing test for AC-3.
2. Run the test command; it fails because `collect` takes no `limit`.
3. Add `limit` to `collect`; run the test command; it passes.

### T3: delete retries a permission error

- Files: `src/app/cli.py`, `tests/test_delete_retry.py`
- Test: `python3 -m unittest tests.test_delete_retry`
- Blocked by: none

1. Write the failing test for AC-4.
2. Run the test command; it fails because `delete` does not retry.
3. Retry `os.remove`; run the test command; it passes.

### T4: report --out never overwrites

- Files: `src/app/cli.py`, `tests/test_report_out.py`
- Test: `python3 -m unittest tests.test_report_out`
- Blocked by: none

1. Write the failing test for AC-5.
2. Run the test command; it fails because `report` takes no `--out`.
3. Add `--out`; run the test command; it passes.

### T5: The audit log reports its failures

- Files: `src/app/audit.py`, `tests/test_audit.py`
- Test: `python3 -m unittest tests.test_audit`
- Blocked by: none

1. Write the failing test for AC-6.
2. Run the test command; it fails because `record` returns True after a failed write.
3. Return False on a failed write or close; run the test command; it passes.

## Review focus

- `src/app/cli.py`: `delete` must still refuse when the audit log cannot be written (T3, T5).
- `tests/`: each boundary and failure branch has its own test, not only the happy path.
