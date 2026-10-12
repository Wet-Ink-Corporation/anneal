# Migrations: expand, migrate, contract

A schema or data change ships in three phases, so that old and new code can run side by side at
every moment and any step can be rolled back by a deploy alone.

| Phase | What changes | Old code | New code | Roll back by |
|---|---|---|---|---|
| Expand | Add the new shape (column, table, field) beside the old; nothing reads it yet | works | works | dropping the unused new shape |
| Migrate | Waves: write both shapes, backfill, then move readers to the new shape | works | works | moving readers back; both shapes are still written |
| Contract | Remove the old shape | breaks | works | a restore: data written only in the new shape is not in the old |

Contract is the one step a deploy cannot undo, so it is gated.

## The wave tracker

Each migrate wave is recorded in the ledger, tagged `migration:<id>`:

| Command | Event | Refused (nothing recorded) when |
|---|---|---|
| `anneal migrate start --migration <id> --wave <n>` | `migrate.wave.started` | wave `n` was already started, or the migration is contracted (exit 3) |
| `anneal migrate done --migration <id> --wave <n>` | `migrate.wave.done` | wave `n` is not open, or the migration is contracted (exit 3) |
| `anneal migrate contract --migration <id>` | `migrate.contract` | any wave is started and not done (exit 1), no wave is done, or it is already contracted (exit 3) |

`--wave` counts from 1, and a wave number is used once. Waves may overlap (wave 2 may start
before wave 1 is done); contract waits for all of them. Two steps recorded at the same moment
cannot both land: the second is refused and is run again.

## A migration, step by step

1. Expand: ship the new shape on its own pull request. Old and new code both still pass their
   tests.
2. For each wave (a dual write, a backfill batch, moving one reader):
   `anneal migrate start --migration users-email --wave 1`, do the wave and check it,
   then `anneal migrate done --migration users-email --wave 1`.
3. When every reader is on the new shape and every wave is done, run
   `anneal migrate contract --migration users-email`. Only after it exits 0 does the pull
   request that removes the old shape merge.
4. `anneal ledger query --tag migration:users-email` lists the waves and the contract in
   order: the migration's record.

## What counts as a wave

| Wave | Done when |
|---|---|
| Dual write | every writer writes both shapes, checked on a sample of new rows |
| Backfill batch | the batch's rows match in both shapes (count and checksum) |
| Reader move | the reader reads the new shape in production and its error rate holds its band (`references/release.md`) |

The default sensitive globs of `anneal ship skip-rule` include `**/migrations/**`, so a
migration's pull requests always get the full review.
