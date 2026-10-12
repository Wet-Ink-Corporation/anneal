<!-- Generated from crates/anneal-cli/src/exit_codes.rs. Do not edit by hand: run
     ANNEAL_BLESS=1 cargo test -p anneal-cli exit_codes_doc to regenerate. -->
# Exit codes

Every `anneal` command exits with a documented code. Codes mean different things for different
commands (`1` is "cycle" for `dag` and "test red" for `task`), so each command has its own
table. A few codes mean the same for every command. Only success exits 0: no failure path exits
0 ("2 never reads as 0"), and the JSON envelope's `ok` is true exactly when the code is 0.

Usage errors keep clap's 2 because Claude Code reads a hook's exit 2 as "block", so a
mis-invoked guard fails closed. A usage error writes nothing on stdout, while a command's own
non-zero code always comes with a JSON envelope on stdout, which tells the two apart. Every
error writes exactly one `ERROR <reason>` line on stderr.

`anneal hook <name>`, the commands `plugin/hooks/hooks.json` runs, is the one exception to "only
success exits 0": hooks fail open. A hook notes its failures in its output (or, when stdout cannot
be written, in one `ERROR` line on stderr) and still exits 0. Its usage errors still exit 2, which
Claude Code reads as "block" on events that can block, so a test runs every `hooks.json` entry's
arguments through the binary.

A guard (`anneal hook guard`, or any hook passed `--fail-closed`) fails closed instead. It denies
what it cannot judge with a deny decision on stdout and exit 0, and when that decision cannot be
written it exits 2 with one `ERROR` line on stderr, so Claude Code blocks the tool call.

## All commands

| Code | Meaning |
|---|---|
| 2 | usage error: bad arguments; nothing on stdout |
| 64 | not implemented yet |
| 70 | internal error: the command could not run |
| 74 | output error: stdout or stderr could not be written |

## `anneal ledger`

| Code | Meaning |
|---|---|
| 0 | ok |
| 2 | schema error |

A green result proves the ledger events read or written match the event schema. It does **not** prove that the events are true, or that the work they record was done well.

## `anneal grill`

| Code | Meaning |
|---|---|
| 0 | ok |
| 1 | invalid design tree |
| 3 | confirm: a question is still open |

A green result proves the design tree is valid; for confirm, no question is open and the human's quoted confirmation of the tree's hash was recorded. It does **not** prove that the right questions were asked, that the answers are right, or that the quoted words came from the human.

## `anneal task done`

| Code | Meaning |
|---|---|
| 0 | tests green |
| 1 | test red |
| 2 | the test command could not be started |

A green result proves the task's tests ran green. It does **not** prove that the tests cover the task's acceptance criteria.

## `anneal plan approve|check`

| Code | Meaning |
|---|---|
| 0 | approved and current |
| 3 | not approved |
| 4 | stale: inputs changed since approval |

A green result proves the plan is approved and current with its inputs. It does **not** prove that the plan is correct or complete.

## `anneal mode`

| Code | Meaning |
|---|---|
| 0 | mode recorded |

A green result proves the mode was recorded for this worktree, with whether its tree was clean. It does **not** prove that the mode's preconditions hold: the preconditions hook checks them at each edit.

## `anneal repro`

| Code | Meaning |
|---|---|
| 0 | record: red, and `repro_red` recorded; leftovers: no `[DEBUG-<id>]` tag left |
| 1 | leftovers: a `[DEBUG-<id>]` tag is left |
| 2 | record: the command could not be started; nothing recorded |
| 3 | record: not red, the command exited 0; nothing recorded |
| 4 | record: timed out, killed; a hang is not a reproduction, nothing recorded |

A green result proves the reproduction exited non-zero and repro_red was recorded for this worktree (record), or no file git knows of holds a [DEBUG-<id>] tag (leftovers). It does **not** prove that the command fails because of the reported bug rather than for another reason (record), or that files over 1 MiB, past the 64 MiB scan budget, with a non-UTF-8 name outside unix, or ignored by git are clean (leftovers).

## `anneal lint`

| Code | Meaning |
|---|---|
| 0 | no errors |
| 1 | error findings |

A green result proves the checked files have no error under anneal's lint rules. It does **not** prove that the content is right, or that its warnings and review items were addressed.

## `anneal dag check|frontier`

| Code | Meaning |
|---|---|
| 0 | ok |
| 1 | cycle |
| 2 | dangling dependency |

A green result proves the task graph has no cycle and no dangling dependency. It does **not** prove that the dependencies are the right ones.

## `anneal floor-guard`

| Code | Meaning |
|---|---|
| 0 | no floor lowered |
| 1 | a floor was lowered |
| 2 | tool error |

A green result proves no quality floor (coverage, lint level, test count) was lowered. It does **not** prove that the floors are high enough.

## `anneal verdict set|valid`

| Code | Meaning |
|---|---|
| 0 | valid |
| 1 | invalid |
| 3 | noise-only |

A green result proves the review verdict is well-formed and backed by findings. It does **not** prove that the reviewer was right.

## `anneal watch pr|ci|deploy`

| Code | Meaning |
|---|---|
| 0 | green |
| 2 | merge conflict |
| 3 | unresolved review threads |
| 4 | failing checks |
| 5 | timeout |
| 6 | gate failed |
| 7 | query failed |

A green result proves the pull request, CI run or deploy reached green within the timeout. It does **not** prove that it stays green, or that its checks test the right things.

## `anneal ship skip-rule`

| Code | Meaning |
|---|---|
| 0 | skip full review |
| 1 | full review |

A green result proves the change meets the rule for skipping a full review. It does **not** prove that the change is free of defects.

## `anneal rollout decide`

| Code | Meaning |
|---|---|
| 0 | go |
| 1 | hold |
| 2 | rollback |

A green result proves the rollout metrics are within their bounds. It does **not** prove that the metrics capture what users experience.

## `anneal perf noise|sample`

| Code | Meaning |
|---|---|
| 0 | keep (sample: saved) |
| 1 | revert: neutral or slower |

A green result proves the change is faster than the baseline beyond run-to-run noise (the 95% interval of the difference is below zero); for sample, its run times were saved. It does **not** prove performance on hardware or workloads that were not measured.

## `anneal migrate`

| Code | Meaning |
|---|---|
| 0 | recorded |
| 1 | contract: a wave is still open; nothing recorded |
| 3 | out of order; nothing recorded |

A green result proves the migration step was recorded in order: a wave started once and done after its start, or the contract with every wave done. It does **not** prove that the waves migrated the data correctly or completely: the ledger records what was reported.

## `anneal brief build`

| Code | Meaning |
|---|---|
| 0 | ok |
| 2 | bad range |

A green result proves the brief was built from the given commit range. It does **not** prove that the brief is complete or its summary accurate.

## `anneal decide`

| Code | Meaning |
|---|---|
| 0 | decided and logged (label: appended) |
| 3 | Clef off for the point; nothing asked or logged |
| 5 | no answer from Clef (unreachable, timeout, bad response or config); apply the fallback |

A green result proves the decision was obtained and recorded. It does **not** prove that the decision is right.

## `anneal calibrate`

| Code | Meaning |
|---|---|
| 0 | ok |

A green result proves the calibration run completed and was recorded. It does **not** prove that the decision point is calibrated well enough to enforce.

## `anneal eval routing`

| Code | Meaning |
|---|---|
| 0 | ok |
| 1 | below bound |
| 2 | invalid fixtures or skills |

A green result proves skill routing accuracy is at or above its bound on the eval set. It does **not** prove routing accuracy on prompts outside the eval set.

## `anneal statusline`

| Code | Meaning |
|---|---|
| 0 | ok |

A green result proves the status line was rendered. It does **not** prove that the state it shows is current.

## `anneal hook`

| Code | Meaning |
|---|---|
| 0 | ran (a guard's deny is on stdout), or failed open with the failure noted |
| 2 | a guard failed closed: its output was not written |

A green result proves the hook ran; anything it could not do is noted in its output. It does **not** prove that the hook succeeded: hooks other than guards fail open, so they exit 0 on failure too; for a guard, that the tool call was allowed.
