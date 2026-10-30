# app

A small command-line tool. Run the tests with `python3 -m unittest`. Run the tool from the repository root with `PYTHONPATH=src`.

- `python3 -m app report <file>` prints a findings report for a log file: its lines containing the marker, `ERROR` unless the `app.ini` beside the log file sets `marker` under `[report]`.
- `python3 -m app tail <n> <file>` prints the last `n` findings of a log file.
- `python3 -m app deleted-tests <diff-file>` prints the test files (`tests/**/*.py`) a `git diff` deletes.
- `python3 -m app delete <path>` deletes a file after writing the deletion to the audit log (`APP_AUDIT_LOG`, default `audit.log`), and refuses when the log cannot be written.
- Plans live in `docs/plans/`, specs in `docs/specs/`.
