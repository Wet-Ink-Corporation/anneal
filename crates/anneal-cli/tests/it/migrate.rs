//! `anneal migrate start|done|contract` through the built binary, on a ledger named by
//! `ANNEAL_STORE` in a scratch directory: a contract while a wave is open is refused and
//! appends nothing; once every wave is done it is recorded.

use std::error::Error;
use std::path::Path;
use std::process::Output;

use serde_json::{Value, json};

use crate::support::anneal_in;

type TestResult = Result<(), Box<dyn Error>>;

/// `anneal migrate <args>` in `dir`, with the ledger at `dir/ledger.sqlite3`.
fn migrate(dir: &Path, args: &[&str]) -> Result<Output, Box<dyn Error>> {
    Ok(anneal_in(dir)
        .env("ANNEAL_STORE", "ledger.sqlite3")
        .arg("migrate")
        .args(args)
        .output()?)
}

fn envelope(output: &Output) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&output.stdout)?)
}

/// The migration's events in the ledger, as `[type, wave]` pairs.
fn events(dir: &Path, migration: &str) -> Result<Vec<Value>, Box<dyn Error>> {
    let output = anneal_in(dir)
        .env("ANNEAL_STORE", "ledger.sqlite3")
        .args([
            "ledger",
            "query",
            "--tag",
            &format!("migration:{migration}"),
        ])
        .output()?;
    let query = envelope(&output)?;
    Ok(query
        .pointer("/data/events")
        .and_then(Value::as_array)
        .map(|events| {
            events
                .iter()
                .map(|e| json!([e.pointer("/event/type"), e.pointer("/event/wave")]))
                .collect()
        })
        .unwrap_or_default())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary and opens SQLite")]
fn a_contract_while_a_wave_is_open_is_refused_and_recorded_once_every_wave_is_done() -> TestResult {
    let dir = tempfile::tempdir()?;
    let wave = |step: &str, n: &str| {
        migrate(
            dir.path(),
            &[step, "--migration", "users-email", "--wave", n],
        )
    };

    let started = [wave("start", "1")?, wave("start", "2")?, wave("done", "1")?];
    let refused = migrate(dir.path(), &["contract", "--migration", "users-email"])?;
    let refusal = envelope(&refused)?;
    let before = events(dir.path(), "users-email")?;
    let finished = wave("done", "2")?;
    let contracted = migrate(dir.path(), &["contract", "--migration", "users-email"])?;

    assert_eq!(
        started.iter().map(|o| o.status.code()).collect::<Vec<_>>(),
        [Some(0), Some(0), Some(0)],
        "the waves are recorded"
    );
    assert_eq!(refused.status.code(), Some(1), "{refusal}");
    assert_eq!(
        (
            &refusal["data"]["recorded"],
            &refusal["data"]["refused"],
            &refusal["data"]["waves"],
            &refusal["findings"][0]["key"],
        ),
        (
            &json!(false),
            &json!("waves_open"),
            &json!([2]),
            &json!("migrate/waves-open:users-email"),
        ),
        "{refusal}"
    );
    assert_eq!(
        before,
        [
            json!(["migrate.wave.started", 1]),
            json!(["migrate.wave.started", 2]),
            json!(["migrate.wave.done", 1]),
        ],
        "the refused contract appended nothing"
    );
    assert_eq!(finished.status.code(), Some(0), "{finished:?}");
    assert_eq!(contracted.status.code(), Some(0), "{contracted:?}");
    assert_eq!(
        events(dir.path(), "users-email")?.last(),
        Some(&json!(["migrate.contract", null])),
        "the contract is recorded last"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary and opens SQLite")]
fn a_refused_step_on_a_missing_ledger_creates_none() -> TestResult {
    let dir = tempfile::tempdir()?;

    let output = migrate(dir.path(), &["done", "--migration", "m", "--wave", "1"])?;
    let refusal = envelope(&output)?;

    assert_eq!(
        (output.status.code(), &refusal["data"]["refused"]),
        (Some(3), &json!("not_open")),
        "{refusal}"
    );
    assert!(
        !dir.path().join("ledger.sqlite3").exists(),
        "no ledger created"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn wave_zero_is_a_usage_error() -> TestResult {
    let dir = tempfile::tempdir()?;

    let output = migrate(dir.path(), &["start", "--migration", "m", "--wave", "0"])?;

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "nothing on stdout");
    Ok(())
}
