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
Re-exported from `lib.rs`: `Store` (`open`, `open_with_busy_timeout`, `append`, `append_if`,
`query`, `tail`), `Filter`,
`Condition`, `Appended`, `Recorded`, `EventEnvelope`, `Timestamp`, `LedgerEvent` and its payload
structs (in `event`), `EventKind`, `REGISTRY_VERSION`, `LedgerTag`, `TagKey`, and the errors
`StoreError`, `OpenError`, `SchemaError`, `TagError`, `TimestampError`, `UnknownEventKind`.
Internal to the workspace (`publish = false`); the wire format (docs/ledger.md) is the contract.

## Invariants
- The ledger never runs in WAL and never creates `-wal` or `-shm` files
  (`the_ledger_runs_under_truncate_and_never_creates_wal_or_shm`).
- Opening only reads until the file is known to be this build's ledger or an empty database,
  so a refused file is left as it was. The schema version and WAL are read from the file
  header before any SQLite connection (`the_header_of_a_real_wal_file_matches_what_sqlite_reports`):
  - a ledger stamped higher than `LEDGER_SCHEMA` (`PRAGMA user_version`) is `NewerSchema`
    (`a_ledger_stamped_by_a_newer_build_is_refused_untouched`,
    `a_newer_ledger_in_wal_is_refused_as_newer_and_left_byte_identical`,
    `the_stamp_after_opening_is_ours_newer_or_unknown`);
  - a file in WAL is `JournalMode`, never converted, and its `-wal` and `-shm` are left as
    they were (`a_wal_ledger_is_refused_and_still_wal_afterwards`,
    `refusing_a_wal_ledger_with_a_live_writer_leaves_its_wal_and_shm_as_they_were`,
    `refusing_a_wal_ledger_creates_no_wal_or_shm_beside_it`);
  - a database with tables but no happenstance schema is `NotALedger`, never given the schema
    (`a_database_that_is_not_a_ledger_is_refused_and_left_byte_identical`).
- `Store` is synchronous: inside a tokio runtime context `open` and every method return
  `StoreError::InsideRuntime` instead of panicking
  (`opening_inside_an_async_runtime_is_refused`,
  `every_operation_inside_an_async_runtime_is_refused`). A `Store` may be dropped anywhere:
  inside a runtime context its runtime shuts down in the background
  (`a_store_dropped_inside_an_async_task_shuts_down_without_panicking`).
- Writes are strict (`EventEnvelope::from_value`): unregistered type, unknown key, bad `ts` or
  tag value is a `SchemaError`. Reads are lenient about keys a newer build added.
- Concurrent appends serialise without loss or corruption
  (`twenty_concurrent_appenders_write_twenty_events_without_corruption`).
- A conditioned append is first-writer-wins (`a_second_approval_of_the_same_plan_conflicts`).
- Every statement on a store's connection waits at most its busy timeout for another
  process's lock: 10 s by default, or the caller's with `Store::open_with_busy_timeout`
  (`a_short_busy_timeout_gives_up_on_a_locked_ledger_quickly`).

## Errors
`SchemaError` (an event does not match the registry), `TagError`, `TimestampError`,
`StoreError` (open, append, read, decode, `InsideRuntime`; `Open` wraps `OpenError` with the
path; `OpenError` includes `NewerSchema`, `JournalMode` and `NotALedger` for refused files). A
condition match is `Appended::Conflict`, not an error.

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
| 2026-10-11 | No WAL-to-TRUNCATE conversion or orphan-WAL handling (weigh-in has them for its 0.1.x ledgers): anneal has no older ledgers; a WAL file is refused with `OpenError::JournalMode`, read before anything is changed | ARC | weigh-in wi-0940cf (refuse WAL); enforced in review round 1 (#54) |
| 2026-10-11 | Only an empty database is given the ledger schema; a database with other tables is refused (`OpenError::NotALedger`), and the CLI reports it like any store that cannot be opened (exit 70) | ARC | review round 1 (#54) |
| 2026-10-11 | `Store` stays synchronous (its own current-thread runtime); calls inside a tokio runtime context return `StoreError::InsideRuntime`; dropped inside one, its runtime is shut down with `shutdown_background` (it only `block_on`s, so nothing is lost) | H-03, ERR-13 | review rounds 1 and 2 (#54) |
| 2026-10-11 | Add `Store::open_with_busy_timeout` (additive) so a caller with a deadline, the SessionStart hook, bounds every statement's wait for a lock; `Store::open` keeps the 10 s default | H-13 | #13 review round 1 (two-way; additive to an internal crate) |
| 2026-10-11 | Registry v2: `grill_confirmed` gains optional `hash` (the confirmed design tree's `sha256:`) and `quote` (the human's words), `#[serde(default)]`, so v1 events still read | API-04 | coordinator default (#16), two-way |
| 2026-10-11 | Registry v3: `test_run` gains optional `exit` (`null` when killed), `duration_ms` and `timeout_secs` (set only when a timeout killed the command), `#[serde(default)]`, so v2 events still read | API-04 | coordinator default (#22), two-way |
| 2026-10-11 | Registry v4: new type `mode_set {mode: build\|diagnose\|off, clean, plan?, tree?}` (`event::Mode`, `event::ModeSet`); its subject key is `worktree` and its value comes from the envelope's `worktree`, not the payload, so `subject_value` is `None` | API-04 | coordinator default (#23), two-way |
| 2026-10-11 | Registry v5: new type `subagent_status {agent, agent_id?, status?, violation?, missing, duration_ms?, tokens?}` (`event::SubagentStatus`, `event::AgentStatus`, `event::TokenUsage`), its subject the new tag key `agent` (`TagKey::Agent`); additive, so v4 events still read. The test `the_registry_is_version_4` is renamed `the_registry_is_version_5` with its expectation (H-12, open for Ryan) | API-04, H-16 | coordinator default (#26), two-way |
| 2026-10-12 | Registry v6: `verdict_set`'s `verdict` gains the levels `tests_verified` and `reviewed` (`Verdict` lists the levels lowest first, then the review outcomes), and the payload gains optional `source` (`VerdictSource`: `local`, `review_plugin`, `human`), `sha`, `patch_id` and `base`, `#[serde(default)]`, so v5 events still read. The test `the_registry_is_version_5` is renamed `the_registry_is_version_6` with its expectation, and `missing_optional_fields_read_as_none`'s `VerdictSet` literal names the four new fields as `None` (assertion unchanged) (H-12, open for Ryan) | API-04, H-16 | coordinator brief (#33), two-way |
| 2026-10-12 | Registry v7: new types `migrate.wave.started {migration, wave}`, `migrate.wave.done {migration, wave}` (`event::MigrateWave`) and `migrate.contract {migration}` (`event::MigrateContract`), stored under the story's dotted names (serde `rename` per variant; happenstance accepts any non-empty type without control characters), their subject the new tag key `migration` (`TagKey::Migration`); additive, so v6 events still read. The test `the_registry_is_version_6` is renamed `the_registry_is_version_7` with its expectation (H-12, open for Ryan). #36 bumps the registry to v7 in parallel: whichever merges second renumbers to v8 | API-04, H-16 | coordinator brief (#35), two-way |
| 2026-10-12 | Registry v8: `round_started` gains optional `ticket` (a fix-loop round of `anneal round`) and its `pr` becomes `Option<u64>` (`#[serde(default)]`, so v7 events still read); the subject tag `pr:` is kept when `pr` is set, and a ticket round has none (found by type and `ticket`, no new tag key). `RoundStarted::pr` changing type is a source break for this crate's users (only `anneal-cli`). The test `the_registry_is_version_7` is renamed `the_registry_is_version_8` with its expectation (#35 took v7 first; H-12, open for Ryan) | API-04, H-16 | coordinator brief (#36), implementer, two-way |
| 2026-10-12 | `RoundStarted`'s fields are private behind `for_pr`, `for_ticket`, `pr()`, `ticket()` and `round()`; an event naming neither a pull request nor a non-empty ticket does not read (`serde(try_from)`, `RoundOfNothing`); `pr` and `ticket` are written only when set, so a pull request round keeps v6's shape. My own unmerged v7 tests now build with the constructors and no longer expect `"pr": null` | API-03, H-12 | #36 pre-review (F6), coordinator |
| 2026-10-12 | Registry v9: new types `weighin_parked {item_id, ticket, decision?}`, `bridge.unavailable {ticket, reason}` and `orchestrate.ruled {ticket, what, why, cost_if_wrong, decision?}` (`event::WeighinParked`, `event::BridgeUnavailable`, `event::OrchestrateRuled`), the last two under the story's dotted names; their subject is the ticket as `task:` (no new tag key); `decision` is written only when set. Additive, so v8 events still read. The test `the_registry_is_version_8` is renamed `the_registry_is_version_9` with its expectation (H-12, expected by the coordinator, open for Ryan) | API-04, H-16 | coordinator brief (#37), implementer, two-way |
| 2026-10-19 | `plan_approved` and `verdict_set` gain an optional `by` (`event::By {actor?: Actor, via?: Via}`, `Actor` `human`/`model`, `Via` `mod`/`bash`/`terminal`; `event/by.rs`), `#[serde(default)]` and written only when set, so v9 events read and a flagless event keeps its shape; its parts are written only when known. Named `by` because the envelope already has `actor`. Not bumped to v10: the bump would rename `the_registry_is_version_9` and change its expectation (and the `registry: 9` the CLI reports), a test change left for Ryan. Struct literals of the two payloads in five test sites (`envelope::tests::missing_optional_fields_read_as_none`, `event::tests::verdict_set_carries_the_level_source_commit_patch_id_and_base`, `store::tests`' `plan_approved` helper, `anneal-cli`'s `verdict::assess::tests` helpers, and `anneal-cli`'s `status/tests.rs`) name `by: None`, their assertions unchanged | API-04, H-12, H-16 | implementer (#62), open for Ryan |
| 2026-10-19 | `By`'s fields are private behind `By::known`, `actor()` and `via()`, `Default` is dropped, and a stored `by: {}` does not read (`serde(try_from)`, `EmptyBy`), so a `by` always names at least one part. The module and `REGISTRY_VERSION` docs state that `by` was added at v9 without a bump, pending Ryan's decision on v10; `the_registry_is_version_9` is unchanged | API-03, API-04 | #62 review (F7, F8), two-way |
