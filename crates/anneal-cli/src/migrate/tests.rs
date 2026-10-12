//! The first-writer-wins append: a step judged on a stale read is refused when another step
//! of the same migration landed after that read, and only then.

use std::path::Path;

use anneal_ledger::{Appended, EventEnvelope, Filter, LedgerTag, Store, TagKey, Timestamp};
use serde_json::{Value, json};

use super::waves::Step;
use super::{Report, Request, read, record};

fn filter(migration: &str) -> Filter {
    Filter {
        types: Vec::new(),
        tags: vec![LedgerTag::new(TagKey::Migration, migration).expect("a valid tag")],
    }
}

fn envelope(migration: &str, step: Step) -> EventEnvelope {
    EventEnvelope::new(
        step.event(migration.to_owned()),
        "test",
        Timestamp::now().expect("a clock"),
    )
}

fn open(dir: &Path) -> Store {
    Store::open(&dir.join("ledger.sqlite3")).expect("opens a new ledger")
}

#[test]
fn a_step_judged_on_a_stale_read_conflicts_reports_concurrent_and_writes_nothing() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = open(dir.path());
    store
        .append(&envelope("m", Step::Start(1)))
        .expect("wave 1 starts");
    let (state, last) = read(&store, &filter("m"), "m").expect("reads the migration");
    // Another process records a step of the same migration after that read.
    store
        .append(&envelope("m", Step::Done(1)))
        .expect("wave 1 is done elsewhere");
    let step = Step::Start(2);

    let appended = record(&store, &envelope("m", step), filter("m"), last).expect("appends");

    assert_eq!(appended, Appended::Conflict, "the later step wins");
    let report = Report {
        request: Request {
            migration: "m",
            step,
            session: None,
        },
        store: dir.path(),
    };
    let mut out = Vec::new();
    let code = report
        .appended(state, appended, &mut out)
        .expect("reports")
        .value();
    let envelope: Value = serde_json::from_slice(&out).expect("one envelope");
    assert_eq!(
        (
            code,
            &envelope["findings"][0]["key"],
            &envelope["data"]["recorded"]
        ),
        (3, &json!("migrate/concurrent:m"), &json!(false)),
        "{envelope}"
    );
    assert_eq!(
        store.query(&filter("m")).expect("reads").len(),
        2,
        "the refused step wrote nothing"
    );
}

#[test]
fn a_step_of_another_migration_after_the_read_does_not_conflict() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = open(dir.path());
    store
        .append(&envelope("m", Step::Start(1)))
        .expect("wave 1 of m starts");
    let (_, last) = read(&store, &filter("m"), "m").expect("reads the migration");
    store
        .append(&envelope("n", Step::Start(1)))
        .expect("wave 1 of n starts in between");

    let appended =
        record(&store, &envelope("m", Step::Done(1)), filter("m"), last).expect("appends");

    assert_eq!(appended, Appended::At(3), "only m's events count");
    assert_eq!(
        store.query(&filter("m")).expect("reads").len(),
        2,
        "m's step is recorded"
    );
}
