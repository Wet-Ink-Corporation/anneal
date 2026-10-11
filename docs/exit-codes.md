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

There is no `anneal hook` command yet (#13). Until it exists, `hooks.json` must not call one: the
usage error would exit 2 and block the session.

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

## `anneal task done`

| Code | Meaning |
|---|---|
| 0 | tests green |
| 1 | test red |

A green result proves the task's tests ran green. It does **not** prove that the tests cover the task's acceptance criteria.

## `anneal plan approve|check`

| Code | Meaning |
|---|---|
| 0 | approved and current |
| 3 | not approved |
| 4 | stale: inputs changed since approval |

A green result proves the plan is approved and current with its inputs. It does **not** prove that the plan is correct or complete.

## `anneal lint`

| Code | Meaning |
|---|---|
| 0 | no findings |
| 1 | findings |

A green result proves the checked files pass anneal's lint rules. It does **not** prove that the content is right; only that it is well-formed.

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

## `anneal perf noise`

| Code | Meaning |
|---|---|
| 0 | keep |
| 1 | revert |

A green result proves the measured change is not a regression beyond run-to-run noise. It does **not** prove performance on hardware or workloads that were not measured.

## `anneal brief build`

| Code | Meaning |
|---|---|
| 0 | ok |
| 2 | bad range |

A green result proves the brief was built from the given commit range. It does **not** prove that the brief is complete or its summary accurate.

## `anneal decide`

| Code | Meaning |
|---|---|
| 0 | ok |
| 5 | Clef unreachable |

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

A green result proves skill routing accuracy is at or above its bound on the eval set. It does **not** prove routing accuracy on prompts outside the eval set.

## `anneal statusline`

| Code | Meaning |
|---|---|
| 0 | ok |

A green result proves the status line was rendered. It does **not** prove that the state it shows is current.
