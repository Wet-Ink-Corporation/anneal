# `anneal task done`

`anneal task done <id> [--timeout <secs>] [--session <id>] [--store <file>] -- <command...>`
records a task as done only when its test command passes.

The command after `--` runs as given, without a shell, in the working directory, with stdin
closed. Its stdout and stderr both go to anneal's stderr, so anneal's stdout stays the one JSON
envelope and nothing is buffered. It has `--timeout` seconds (default 1800, at least 1); past
that it is killed.

| Exit | Meaning | Ledger |
|---|---|---|
| 0 | the command exited 0 | `test_run` (`outcome: green`, `exit: 0`), then `task_done` |
| 1 | the command exited non-zero, was killed by a signal, or ran past `--timeout` | `test_run` (`outcome: red`) only |
| 2 | the command could not be started (not found, not executable), or a usage error | nothing; no ledger is opened |

A non-zero result never records `task_done`. The ledger is opened only after the command has
finished, so a command that cannot start leaves no file. The task id and `--session` are
checked as ledger tags before the command runs: one the ledger would refuse is a usage error
and nothing runs. A failure to wait on the command, or to write the ledger, exits 70.

`test_run` carries `task`, `command` (the words joined by spaces, for reading rather than
re-running), `outcome`, `exit` (`null` when the command was killed), `duration_ms` (from a
monotonic clock) and `timeout_secs` (set only when the timeout killed it); see
`docs/ledger.md`. The envelope's `data` holds `task`, `command`, `store`, `source` and
`events` (each with its ledger `position`); a red run has one `task/red:<id>` finding.

Limit: on a timeout only the command's own process is killed. A process it started (a test
binary under `cargo test`) can outlive it; std has no portable process-group kill.
