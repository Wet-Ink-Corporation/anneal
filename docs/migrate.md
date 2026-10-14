# Migration wave tracker

```text
anneal migrate start    --migration <id> --wave <n> [--session <id>]
anneal migrate done     --migration <id> --wave <n> [--session <id>]
anneal migrate contract --migration <id> [--session <id>]
```

A schema or data migration ships as expand, migrate, contract
(`plugin/skills/ship/references/migrate.md`). The migrate phase runs in waves, and the contract
step, the one a deploy cannot undo, waits for all of them. `anneal migrate` records each step in
the ledger and refuses one out of order.

## Events

| Command | Event (registry v7) | Refused (nothing appended) |
|---|---|---|
| `start` | `migrate.wave.started {migration, wave}` | the wave was already started (3); the migration is contracted (3) |
| `done` | `migrate.wave.done {migration, wave}` | the wave is not open (3); the migration is contracted (3) |
| `contract` | `migrate.contract {migration}` | a wave is started and not done (**1**); no wave is done (3); already contracted (3) |

Every event is tagged `migration:<id>`, so `anneal ledger query --tag migration:<id>` lists a
migration's history. `--wave` counts from 1 and each number is used once; waves may overlap.

## Rules

- The state is folded from the migration's events in ledger order (at most 4,096; more is an
  error, 70, rather than a partial fold).
- The append is first-writer-wins (`Store::append_if`): it succeeds only if no event of the
  migration landed after the last one read, so two racing steps cannot both be recorded; the
  loser is refused (`concurrent`, 3) and is run again.
- A `--migration` or `--session` the ledger cannot tag is a usage error (2) that opens no ledger.
  A refused step on a ledger that does not exist yet creates none.
- Like `mode` and `verdict`, it has no `--store`: it writes to the ledger `$ANNEAL_STORE`, the git
  common dir or the folder gives (`docs/ledger.md`).

## Output

`data` holds `migration`, `wave` (for `start` and `done`), `recorded`, `position` (when
recorded), `store`, and the migration's state: `open` and `done` (wave numbers, ascending) and
`contracted`, after the step when it was recorded, as read when it was refused. A refusal adds
`refused` (`waves_open` with `waves`, `no_wave_done`, `already_started` or `not_open` with
`wave`, `contracted`, `concurrent`) and one finding, `migrate/<kind>:<migration>`, naming the
command that moves the migration on.

Exit codes: 0 recorded; 1 contract refused while a wave is open; 3 any other refusal; 2 usage.
