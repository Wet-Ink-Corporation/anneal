# The four tiers a skill passes before merge

`evals/README.md` in the anneal repository holds each tier's schema, statistics and exit codes.

| Tier | Command | Bar | Applies to |
|---|---|---|---|
| 0 | `anneal lint skills plugin/skills --index docs/bootstrap-index.md`, `claude plugin validate --strict plugin` | zero errors | every skill |
| 1 | `anneal eval routing`, with `evals/routing/<skill>.yaml` | 100% rank-1 | model-invoked skills |
| 2 | `anneal eval routing --router claude --changed-since <base>` | every prompt's should-fire lower bound 0.70 or more; should-not upper bound 0.20 or less | model-invoked skills |
| 3 | `anneal eval uplift --skill <name>`, over `evals/uplift/<skill>/` | uplift lower bound 0 or more, every guard pass^3, no regression suite below 0.90 | every skill with a suite; each entry skill must have one |

A user-invoked skill (`disable-model-invocation: true`) is never routed by the model, so Tiers 1
and 2 do not apply to it and it has no routing fixture.

## Cost

Tiers 2 and 3 run `claude plugin eval` and spend money. `--max-cost-usd` caps the start of each
batch (25 US dollars by default) and `--timeout` the whole run (3600 s). The plugin's hooks run on
the host during the eval, so run it in a container. Ask the human before each run;
`anneal eval uplift --check` and Tiers 0 and 1 cost nothing.

## The no-guidance control

The baseline arm of `anneal eval uplift` runs each case against the plugin with the skill's
directory removed (or `--baseline <plugin dir>`). For a new skill that is the control: the case
must fail there, or the skill has nothing to add. A grader that passes in 90% or more of baseline
runs is listed as non-discriminating; rewrite or drop it.
