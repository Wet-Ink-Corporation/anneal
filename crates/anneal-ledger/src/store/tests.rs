use std::path::{Path, PathBuf};

use happenstance_core::Event;
use happenstance_sqlite::rusqlite::Connection;

use super::schema::{LEDGER_SCHEMA, connect, user_version};
use super::{Appended, Condition, Filter, OpenError, Recorded, Store, StoreError};
use crate::envelope::{EventEnvelope, Timestamp};
use crate::event::{EventKind, LedgerEvent, PlanApproved, TaskDone, TestOutcome, TestRun};
use crate::tag::{LedgerTag, TagKey};

fn ts() -> Timestamp {
    Timestamp::try_from("2026-10-11T09:30:00Z".to_owned()).expect("a valid timestamp")
}

fn in_session(event: LedgerEvent, session: &str) -> EventEnvelope {
    let mut envelope = EventEnvelope::new(event, "test", ts());
    envelope.session_id = Some(session.to_owned());
    envelope
}

fn task_done(task: &str, session: &str) -> EventEnvelope {
    in_session(
        LedgerEvent::TaskDone(TaskDone {
            task: task.to_owned(),
        }),
        session,
    )
}

fn test_run(task: &str, session: &str) -> EventEnvelope {
    in_session(
        LedgerEvent::TestRun(TestRun {
            task: Some(task.to_owned()),
            command: "cargo test".to_owned(),
            outcome: TestOutcome::Green,
        }),
        session,
    )
}

fn plan_approved(plan: &str, hash: &str) -> EventEnvelope {
    in_session(
        LedgerEvent::PlanApproved(PlanApproved {
            plan: plan.to_owned(),
            hash: hash.to_owned(),
        }),
        "s1",
    )
}

fn ledger_path(dir: &Path) -> PathBuf {
    dir.join("anneal/ledger.sqlite3")
}

/// `<ledger><suffix>`: the journal, WAL and shared-memory files SQLite keeps beside a database.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn positions(events: &[Recorded]) -> Vec<u64> {
    events.iter().map(|recorded| recorded.position).collect()
}

fn tag(text: &str) -> LedgerTag {
    text.parse().expect("a vocabulary tag")
}

/// A plain SQLite connection that changes nothing on open, for setting up and inspecting files.
fn raw(path: &Path) -> Connection {
    Connection::open(path).expect("SQLite opens the file")
}

/// The journal mode the file at `path` is in, as SQLite reports it.
fn journal_mode_of(path: &Path) -> String {
    raw(path)
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("readable journal mode")
}

/// A ledger at `path` switched to WAL (as another tool, or happenstance's own `open`, would).
fn wal_ledger(path: &Path, user_version: i64) {
    drop(Store::open(path).expect("opens"));
    let conn = raw(path);
    conn.execute_batch(&format!("PRAGMA user_version = {user_version}"))
        .expect("stamps");
    let mode: String = conn
        .query_row("PRAGMA journal_mode = wal", [], |row| row.get(0))
        .expect("switches");
    assert_eq!(mode, "wal", "set up in WAL");
}

#[test]
fn every_registered_kind_is_a_valid_store_event_type() {
    for kind in EventKind::ALL {
        assert!(
            Event::new(kind.as_str(), &b"{}"[..]).is_ok(),
            "{kind} is a valid happenstance event type"
        );
    }
}

#[test]
#[cfg_attr(miri, ignore = "opens SQLite through FFI")]
fn an_appended_event_reads_back_at_position_1() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = Store::open(&ledger_path(dir.path())).expect("opens");
    let event = task_done("T1", "s1");

    let position = store.append(&event).expect("appends");
    let events = store.query(&Filter::default()).expect("reads");

    assert_eq!(position, 1, "the first event is at 1");
    assert_eq!(events, [Recorded { position: 1, event }], "reads back");
}

#[test]
#[cfg_attr(miri, ignore = "opens SQLite through FFI")]
fn query_filters_by_type_tag_and_session() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = Store::open(&ledger_path(dir.path())).expect("opens");
    for event in [
        task_done("T1", "s1"),
        test_run("T1", "s2"),
        task_done("T2", "s2"),
    ] {
        store.append(&event).expect("appends");
    }

    let by_type = Filter {
        types: vec![EventKind::TaskDone],
        ..Filter::default()
    };
    let by_task = Filter {
        tags: vec![tag("task:T1")],
        ..Filter::default()
    };
    let by_session_and_type = Filter {
        types: vec![EventKind::TaskDone, EventKind::TestRun],
        tags: vec![LedgerTag::new(TagKey::Session, "s2").expect("valid")],
    };

    assert_eq!(
        positions(&store.query(&by_type).expect("reads")),
        [1, 3],
        "type"
    );
    assert_eq!(
        positions(&store.query(&by_task).expect("reads")),
        [1, 2],
        "tag"
    );
    assert_eq!(
        positions(&store.query(&by_session_and_type).expect("reads")),
        [2, 3],
        "types are any-of, tags all-of"
    );
}

#[test]
#[cfg_attr(miri, ignore = "opens SQLite through FFI")]
fn tail_returns_the_last_n_in_position_order() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = Store::open(&ledger_path(dir.path())).expect("opens");
    for task in ["T1", "T2", "T3", "T4"] {
        store.append(&task_done(task, "s1")).expect("appends");
    }

    let last_two = store.tail(&Filter::default(), 2).expect("reads");
    let more_than_all = store.tail(&Filter::default(), 10).expect("reads");

    assert_eq!(positions(&last_two), [3, 4], "oldest first");
    assert_eq!(positions(&more_than_all), [1, 2, 3, 4], "n past the end");
}

#[test]
#[cfg_attr(miri, ignore = "opens SQLite through FFI")]
fn a_second_approval_of_the_same_plan_conflicts() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = Store::open(&ledger_path(dir.path())).expect("opens");
    let first_approval_of = |plan: &str| {
        Condition::none_matching(Filter {
            types: vec![EventKind::PlanApproved],
            tags: vec![LedgerTag::new(TagKey::Plan, plan).expect("valid")],
        })
    };

    let first = store.append_if(&plan_approved("p.md", "h1"), &first_approval_of("p.md"));
    let second = store.append_if(&plan_approved("p.md", "h2"), &first_approval_of("p.md"));
    let other = store.append_if(&plan_approved("q.md", "h1"), &first_approval_of("q.md"));

    assert_eq!(
        first.expect("appends"),
        Appended::At(1),
        "first writer wins"
    );
    assert_eq!(
        second.expect("no error"),
        Appended::Conflict,
        "second loses"
    );
    assert_eq!(
        other.expect("appends"),
        Appended::At(2),
        "another plan is free"
    );
    assert_eq!(
        store.query(&Filter::default()).expect("reads").len(),
        2,
        "the conflicting event was not written"
    );
}

#[test]
#[cfg_attr(miri, ignore = "opens SQLite through FFI")]
fn a_condition_after_a_position_ignores_what_the_writer_already_saw() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = Store::open(&ledger_path(dir.path())).expect("opens");
    store.append(&task_done("T1", "s1")).expect("appends");
    let unchanged_since_1 = Condition::none_matching(Filter {
        tags: vec![tag("task:T1")],
        ..Filter::default()
    })
    .after(1);

    let first = store.append_if(&test_run("T1", "s1"), &unchanged_since_1);
    let stale = store.append_if(&test_run("T1", "s2"), &unchanged_since_1);

    assert_eq!(first.expect("appends"), Appended::At(2), "nothing after 1");
    assert_eq!(stale.expect("no error"), Appended::Conflict, "2 is after 1");
}

#[test]
#[cfg_attr(miri, ignore = "opens SQLite through FFI")]
fn the_ledger_runs_under_truncate_and_never_creates_wal_or_shm() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = ledger_path(dir.path());
    let no_wal_files = |when: &str| {
        assert!(!sibling(&path, "-wal").exists(), "no -wal {when}");
        assert!(!sibling(&path, "-shm").exists(), "no -shm {when}");
    };

    let store = Store::open(&path).expect("opens");
    store.append(&task_done("T1", "s1")).expect("appends");
    let settings = store.inner.settings().expect("readable settings");
    no_wal_files("while open");
    drop(store);
    let reopened = Store::open(&path).expect("reopens");
    assert_eq!(
        reopened.query(&Filter::default()).expect("reads").len(),
        1,
        "kept"
    );
    drop(reopened);

    assert_eq!(settings.journal_mode(), "truncate", "TRUNCATE journal");
    no_wal_files("after close");
    let journal = std::fs::metadata(sibling(&path, "-journal")).expect("the journal is kept");
    assert_eq!(journal.len(), 0, "truncated, not deleted");
}

#[test]
#[cfg_attr(miri, ignore = "opens SQLite through FFI")]
fn a_new_ledger_is_stamped_with_the_schema_version() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = ledger_path(dir.path());

    drop(Store::open(&path).expect("opens"));

    let conn = connect(&path).expect("connects");
    assert_eq!(
        user_version(&conn).expect("readable"),
        LEDGER_SCHEMA,
        "stamped"
    );
}

#[test]
#[cfg_attr(miri, ignore = "opens SQLite through FFI")]
fn a_ledger_stamped_by_a_newer_build_is_refused_untouched() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = ledger_path(dir.path());
    drop(Store::open(&path).expect("opens"));
    let newer = LEDGER_SCHEMA + 1;
    connect(&path)
        .expect("connects")
        .execute_batch(&format!("PRAGMA user_version = {newer}"))
        .expect("stamps");
    let before = std::fs::read(&path).expect("readable");

    let result = Store::open(&path);

    assert!(
        matches!(
            &result,
            Err(StoreError::Open { source: OpenError::NewerSchema { found, .. }, .. }) if *found == newer
        ),
        "{result:?}"
    );
    assert_eq!(std::fs::read(&path).expect("readable"), before, "untouched");
}

#[test]
#[cfg_attr(miri, ignore = "opens SQLite through FFI")]
fn a_database_that_is_not_a_ledger_is_refused_and_left_byte_identical() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("notes.sqlite3");
    raw(&path)
        .execute_batch("CREATE TABLE notes (body TEXT); INSERT INTO notes VALUES ('keep me');")
        .expect("a foreign database");
    let before = std::fs::read(&path).expect("readable");

    let result = Store::open(&path);

    assert!(
        matches!(
            &result,
            Err(StoreError::Open {
                source: OpenError::NotALedger,
                ..
            })
        ),
        "{result:?}"
    );
    assert_eq!(
        std::fs::read(&path).expect("readable"),
        before,
        "byte-identical"
    );
}

#[test]
#[cfg_attr(miri, ignore = "opens SQLite through FFI")]
fn a_wal_ledger_is_refused_and_still_wal_afterwards() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = ledger_path(dir.path());
    wal_ledger(&path, LEDGER_SCHEMA);

    let result = Store::open(&path);

    assert!(
        matches!(
            &result,
            Err(StoreError::Open { source: OpenError::JournalMode { found }, .. }) if found == "wal"
        ),
        "{result:?}"
    );
    assert_eq!(journal_mode_of(&path), "wal", "not converted");
}

#[test]
#[cfg_attr(miri, ignore = "opens SQLite through FFI")]
fn a_newer_ledger_in_wal_is_refused_as_newer_and_left_byte_identical() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = ledger_path(dir.path());
    let newer = LEDGER_SCHEMA + 1;
    wal_ledger(&path, newer);
    let before = std::fs::read(&path).expect("readable");

    let result = Store::open(&path);

    assert!(
        matches!(
            &result,
            Err(StoreError::Open { source: OpenError::NewerSchema { found, .. }, .. }) if *found == newer
        ),
        "{result:?}"
    );
    assert_eq!(
        std::fs::read(&path).expect("readable"),
        before,
        "byte-identical"
    );
    assert_eq!(journal_mode_of(&path), "wal", "not converted");
}

#[test]
#[cfg_attr(miri, ignore = "opens SQLite through FFI")]
fn opening_inside_an_async_runtime_is_refused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = ledger_path(dir.path());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("a runtime starts");

    let result = runtime.block_on(async { Store::open(&path).map(drop) });

    assert!(
        matches!(result, Err(StoreError::InsideRuntime)),
        "{result:?}"
    );
}

#[test]
#[cfg_attr(miri, ignore = "opens SQLite through FFI")]
fn every_operation_inside_an_async_runtime_is_refused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = Store::open(&ledger_path(dir.path())).expect("opens");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("a runtime starts");
    let anything = Condition::none_matching(Filter::default());

    let results = runtime.block_on(async {
        [
            store.append(&task_done("T1", "s1")).map(drop),
            store.append_if(&task_done("T1", "s1"), &anything).map(drop),
            store.query(&Filter::default()).map(drop),
            store.tail(&Filter::default(), 1).map(drop),
        ]
    });

    for result in results {
        assert!(
            matches!(result, Err(StoreError::InsideRuntime)),
            "{result:?}"
        );
    }
    assert_eq!(
        store.query(&Filter::default()).expect("reads").len(),
        0,
        "nothing was written"
    );
}

#[test]
#[cfg_attr(miri, ignore = "opens SQLite through FFI")]
fn twenty_concurrent_appenders_write_twenty_events_without_corruption() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = ledger_path(dir.path());
    // Every appender opens, then waits for the others, so the twenty appends start together. A
    // failed open still reaches the barrier, so no thread waits forever.
    let start = std::sync::Barrier::new(20);

    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..20)
            .map(|n| {
                let (path, start) = (&path, &start);
                std::thread::Builder::new()
                    .name(format!("appender-{n}"))
                    .spawn_scoped(scope, move || {
                        let store = Store::open(path);
                        start.wait();
                        store?.append(&task_done(&format!("T{n}"), "s1"))
                    })
                    .expect("the OS starts the thread")
            })
            .collect();
        for handle in handles {
            handle
                .join()
                .expect("the appender did not panic")
                .expect("the append succeeded");
        }
    });

    let events = Store::open(&path)
        .expect("opens")
        .query(&Filter::default())
        .expect("reads");
    let integrity: String = connect(&path)
        .expect("connects")
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .expect("checks");
    assert_eq!(
        positions(&events),
        (1..=20).collect::<Vec<_>>(),
        "20 events"
    );
    assert_eq!(integrity, "ok", "no corruption");
}
