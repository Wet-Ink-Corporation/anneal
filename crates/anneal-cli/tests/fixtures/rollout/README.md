# Rollout band fixtures

`bands.json` holds three metrics: `error_rate` and `p99_ms` (lower is better) and
`success_rate` (higher is better). The metric files sit at the exact boundaries:

| File | Values | `anneal rollout decide` |
|---|---|---|
| `go.json` | just inside every go band | 0 go |
| `hold.json` | exactly at every hold threshold | 1 hold |
| `rollback.json` | exactly at every rollback threshold | 2 rollback |

A threshold belongs to the worse band, so a value exactly at `hold` holds and one exactly at
`rollback` rolls back. See `docs/rollout.md`.
