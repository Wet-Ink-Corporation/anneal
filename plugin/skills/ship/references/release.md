# Release: rollout bands and error budgets

A rollout widens in stages. Before each stage, `anneal rollout decide` judges the current
metrics against the bands: exit 0 go (widen), 1 hold (stay, investigate), 2 rollback (run the
rollback plan now). The decision is the worst metric's band, and a metric with no value holds.

## Stages

| Stage | Traffic | Soak before the next stage |
|---|---|---|
| 1 | 1% | 30 minutes |
| 2 | 5% | 1 hour |
| 3 | 25% | 2 hours |
| 4 | 50% | 4 hours |
| 5 | 100% | 24 hours before the rollout is closed |

Run `anneal rollout decide` at the end of each soak, and again at once on any page.

## Default bands

A threshold belongs to the worse band: exactly at `hold` holds, exactly at `rollback` rolls
back. Compare the canary with the baseline it replaces, over the same window.

| Metric | Better | Go | Hold | Rollback |
|---|---|---|---|---|
| `error_rate` (5xx / requests) | lower | below 0.5% | 0.5% up to 2% | 2% or more |
| `p99_latency_ratio` (canary p99 / baseline p99) | lower | below 1.10 | 1.10 up to 1.25 | 1.25 or more |
| `success_rate` (good / all) | higher | above 99.9% | 99.9% down to 99.5% | 99.5% or less |
| `budget_burn_rate` (1-hour burn rate, below) | lower | below 2 | 2 up to 14.4 | 14.4 or more |

As a bands file (`--bands`):

```json
{
  "metrics": {
    "error_rate": {"better": "lower", "hold": 0.005, "rollback": 0.02},
    "p99_latency_ratio": {"better": "lower", "hold": 1.10, "rollback": 1.25},
    "success_rate": {"better": "higher", "hold": 0.999, "rollback": 0.995},
    "budget_burn_rate": {"better": "lower", "hold": 2, "rollback": 14.4}
  }
}
```

```sh
anneal rollout decide --bands bands.json \
  --metrics '{"error_rate": 0.003, "p99_latency_ratio": 1.04, "success_rate": 0.9994, "budget_burn_rate": 1.2}'
```

## Error budget

The error budget is `1 - SLO` of the window (a 99.9% monthly SLO allows 0.1% bad requests, about
43 minutes of full outage). The burn rate is how fast it is spent: 1 spends exactly the budget
over the window.

| Budget left in the window | Rollouts allowed |
|---|---|
| 50% or more | all, at the stages above |
| 25% up to 50% | one stage at a time, double soaks |
| above 0% up to 25% | fixes and rollbacks only |
| 0% | rollbacks and reliability work only, until the window recovers |

| Burn rate (over 1 hour) | Meaning | `rollout decide` band |
|---|---|---|
| below 2 | normal spend | go |
| 2 up to 14.4 | the month's budget gone in 15 days or less | hold |
| 14.4 or more | 2% of the month's budget gone in one hour | rollback |
