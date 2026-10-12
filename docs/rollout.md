# Rollout decisions

```text
anneal rollout decide --metrics '<json object of numbers>' --bands <file>
```

`rollout decide` judges a rollout's metrics against band tables and exits with the decision:
**0 go**, **1 hold**, **2 rollback**. The `ship` skill runs it before each rollout stage
(`plugin/skills/ship/references/release.md` holds the default stages, bands and error-budget
policy).

## Bands file

JSON, at most 1 MiB and 256 metrics, read strictly (an unknown key is refused):

```json
{
  "metrics": {
    "error_rate": {"better": "lower", "hold": 0.01, "rollback": 0.05},
    "success_rate": {"better": "higher", "hold": 0.999, "rollback": 0.99}
  }
}
```

- `better` is `lower` (error rates, latency) or `higher` (success rates, availability).
- `hold` and `rollback` are finite numbers, and `hold` is reached first: for `lower`,
  `hold <= rollback`; for `higher`, `hold >= rollback`. Equal thresholds leave no hold band.

## Rules

| `better` | go | hold | rollback |
|---|---|---|---|
| `lower` | value < `hold` | `hold` <= value < `rollback` | value >= `rollback` |
| `higher` | value > `hold` | `rollback` < value <= `hold` | value <= `rollback` |

- **A threshold belongs to the worse band.** A value exactly at `hold` holds; exactly at
  `rollback` it rolls back.
- **Missing is hold.** A metric the bands name but `--metrics` does not give holds (finding
  `rollout/missing:<metric>`): no evidence is not a go.
- **Worst band wins.** Any rollback metric gives 2; else any hold gives 1; else 0.
- Metrics no band names are listed in `data.unbanded` and change nothing.

The boundary fixtures are `crates/anneal-cli/tests/fixtures/rollout/` (`go.json`, `hold.json`,
`rollback.json` against `bands.json`).

## Output

`data` holds `decision` (`go`, `hold`, `rollback`), `metrics` (each banded metric by name:
`metric`, `value` (`null` when missing), `rating`, `better`, `hold`, `rollback`) and `unbanded`.
Each metric that is not go is a finding: `rollout/hold:<metric>`, `rollout/rollback:<metric>`
or `rollout/missing:<metric>`.

Exit codes: 0 go, 1 hold, 2 rollback (each with its envelope on stdout); a bands file that is
missing, not a regular file, too large or invalid, or `--metrics` that is not a JSON object of
numbers, is a usage error (2 with one `ERROR` line and nothing on stdout); a bands file that
exists but cannot be read is 70. A rollback's 2 always comes with an envelope, a usage error's
never.
