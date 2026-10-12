# Performance noise

```text
anneal perf sample [-n <N>] [--timeout <secs>] --out <file> -- <command> [args...]
anneal perf noise  [-n <N>] [--timeout <secs>] --baseline <file> -- <command> [args...]
```

`perf` tells a performance change from run-to-run noise. Measure the baseline before the change
with `sample`, then the candidate after it with `noise`, on the same machine and workload:

```sh
git switch main      && anneal perf sample -n 20 --out /tmp/base.json -- ./target/release/app bench
git switch my-branch && anneal perf noise  -n 20 --baseline /tmp/base.json -- ./target/release/app bench
```

## Runs

- The command runs as given (no shell), `-n` times (2 to 100, default 10), each in the working
  directory with stdin, stdout and stderr null.
- A run is timed by the wall clock from start to exit, checked every millisecond, so the
  resolution is about 1 ms: time a command that runs for well over that.
- Each run has `--timeout` seconds (1 to 3600, default 300); past it the run is killed.
- A run that exits non-zero, is killed by a signal or times out stops the command (exit 70): its
  time would measure a failure. A command that cannot start is a usage error (2).

## The decision

`noise` compares the candidate's run times with the baseline's by Welch's t interval on the
difference of means (candidate minus baseline) at 95%. The t value is the 97.5th percentile of
Student's t at the Welch–Satterthwaite degrees of freedom, read from a table at the largest
tabled df not above it, so the interval is never narrower than the exact one.

| Interval | Verdict | Decision | Exit |
|---|---|---|---|
| wholly below 0 | `faster` | keep | 0 |
| holds 0 | `neutral` | revert | 1 |
| wholly above 0 | `slower` | revert | 1 |

**Neutral means revert**: a change that cannot be told from noise does not earn its keep.

## Files and output

The sample file is `{"version": 1, "command": [...], "samples_secs": [...]}` (at most 1 MiB
read; another version, fewer than 2 or more than 100 runs, or a time outside 0 to 86,400 s is a
usage error).

`sample`'s `data` holds `out`, `runs`, `mean_secs` and `samples_secs`. `noise`'s holds
`decision`, `verdict`, `confidence` (0.95), `delta_secs`, `low_secs`, `high_secs`, `df`, `t`,
`baseline` (`file`, `command`, `runs`, `mean_secs`) and `candidate` (`runs`, `mean_secs`,
`samples_secs`); a revert carries one finding, `perf/neutral` or `perf/slower`.

Exit codes: 0 keep (and `sample`: saved), 1 revert; 2 usage (bad arguments, a missing or
invalid baseline, a command that cannot start); 70 (a failed run, a baseline that exists but
cannot be read, a sample file that cannot be written).

The decision is tested with a fake runner and seeded run times (`perf::tests`,
`perf::stats::tests`), never by timing real processes.
