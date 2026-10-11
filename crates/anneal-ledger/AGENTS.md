# Crate charter: `anneal-ledger`

## Purpose
anneal's event ledger: the versioned registry of event types, the envelope they are stored in,
and happenstance's SQLite event store opened under a TRUNCATE journal. Commands and hooks
append and query through it in-process. It does not resolve where the store lives (git, the
environment, flags): callers pass a path.

## Layer
adapter (SQLite through happenstance) with its own small domain (`event`, `tag`, `envelope`).
The domain modules do no I/O; `store` is the only module that touches the file system.

## Allowed dependencies
- Workspace crates: none.
- Third-party: `happenstance-core` 0.3.2 (`std`, `serde`) and `happenstance-sqlite` 0.3.2
  (`event-store`): the event store, pinned with weigh-in; `tokio` (`rt`): a current-thread
  runtime drives happenstance's async API; `time` (`formatting`, `parsing`): RFC 3339 `ts`;
  `tempfile`: the schema template on the local temporary disk; `serde`, `serde_json`,
  `thiserror`.
- Never: `clap`, `anyhow`, git or environment access, anything that needs a network at build
  time.

## Public surface
Re-exported from `lib.rs`: `Store` (`open`, `append`, `append_if`, `query`, `tail`), `Filter`,
`Condition`, `Appended`, `Recorded`, `EventEnvelope`, `Timestamp`, `LedgerEvent` and its payload
structs (in `event`), `EventKind`, `REGISTRY_VERSION`, `LedgerTag`, `TagKey`, and the errors
`StoreError`, `OpenError`, `SchemaError`, `TagError`, `TimestampError`, `UnknownEventKind`.
Internal to the workspace (`publish = false`); the wire format (docs/ledger.md) is the contract.

## Invariants
- The ledger never runs in WAL and never creates `-wal` or `-shm` files
  (`the_ledger_runs_under_truncate_and_never_creates_wal_or_shm`).
- `PRAGMA user_version` is `LEDGER_SCHEMA`; a ledger stamped higher is refused untouched
  (`a_ledger_stamped_by_a_newer_build_is_refused_untouched`).
- Writes are strict (`EventEnvelope::from_value`): unregistered type, unknown key, bad `ts` or
  tag value is a `SchemaError`. Reads are lenient about keys a newer build added.
- Concurrent appends serialise without loss or corruption
  (`twenty_concurrent_appenders_write_twenty_events_without_corruption`).
- A conditioned append is first-writer-wins (`a_second_approval_of_the_same_plan_conflicts`).

## Errors
`SchemaError` (an event does not match the registry), `TagError`, `TimestampError`,
`StoreError` (open, append, read, decode; `Open` wraps `OpenError` with the path). A condition
match is `Appended::Conflict`, not an error.

## Test strategy
Unit tests beside the code; store tests in `src/store/tests.rs` open real ledgers in temporary
directories. Run with `cargo test -p anneal-ledger`.

## Do not
- Do not open the ledger with `SqliteEventStore::open` (it switches to WAL).
- Do not rename or remove an event type or payload field without a migration (H-16); add
  optional fields with `#[serde(default)]` and bump `REGISTRY_VERSION`.
- Do not move happenstance off 0.3.2 without a schema migration step (H-16).

## Decisions
| Date | Decision | Rule | Decided by |
|---|---|---|---|
| 2026-10-11 | New crate for the ledger: a different dependency profile (bundled SQLite, tokio) from the CLI, and a reuse boundary for hooks and later commands | ARC-07 | coordinator brief (#11) |
| 2026-10-11 | Add happenstance-core and happenstance-sqlite 0.3.2, pinned with weigh-in; dependabot ignores `happenstance-*` | H-16 | story #11 |
| 2026-10-11 | Add tokio (`rt` only) to drive happenstance's async API from synchronous code | H-16 | story #11 (as weigh-in) |
| 2026-10-11 | Add time (`formatting`, `parsing`) for RFC 3339 timestamps | H-16 | story #11 (as weigh-in) |
| 2026-10-11 | tempfile as a normal dependency: the schema template is built on the local temporary disk (an in-memory database cannot be attached from another connection) | H-16 | `.claude/temper.json` pre-approved |
| 2026-10-11 | No WAL-to-TRUNCATE conversion or orphan-WAL handling (weigh-in has them for its 0.1.x ledgers): anneal has no older ledgers; a WAL file is refused with `OpenError::JournalMode` | ARC | implementer (#11), open for review |
