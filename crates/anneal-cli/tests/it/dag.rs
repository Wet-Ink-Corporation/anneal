//! `anneal dag check|frontier` through the built binary, over the markdown graph fixtures in
//! `tests/fixtures/dag/` (acyclic, cycle, dangling, empty).

use std::error::Error;
use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};

use crate::support::anneal_with_home;

type TestResult = Result<(), Box<dyn Error>>;

fn fixtures() -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "tests/fixtures/dag"]
        .iter()
        .collect()
}

/// Runs `anneal dag <args>` in the fixtures directory.
fn dag(args: &[&str]) -> Result<Output, Box<dyn Error>> {
    let home = tempfile::tempdir()?;
    Ok(anneal_with_home(&fixtures(), home.path())
        .arg("dag")
        .args(args)
        .output()?)
}

fn envelope(output: &Output) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn finding_messages(envelope: &Value) -> Vec<&str> {
    envelope["findings"]
        .as_array()
        .map(|findings| {
            findings
                .iter()
                .filter_map(|f| f["message"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn check_exits_0_on_the_acyclic_fixture() -> TestResult {
    let output = dag(&["check", "--dir", "acyclic"])?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let envelope = envelope(&output)?;
    assert_eq!(envelope["ok"], json!(true), "ok");
    assert_eq!(
        envelope["data"],
        json!({"tracker": "markdown", "tickets": 4, "done": 1, "edges": 4, "dangling": [], "cycle": []}),
        "README.md is not a ticket"
    );
    assert!(output.stderr.is_empty(), "nothing on stderr");
    Ok(())
}

#[test]
fn check_exits_1_naming_the_cycle_in_order() -> TestResult {
    let output = dag(&["check", "--dir", "cycle"])?;

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let envelope = envelope(&output)?;
    assert_eq!(
        envelope["data"]["cycle"],
        json!(["a", "c", "b"]),
        "a by c, c by b, b by a"
    );
    assert_eq!(
        finding_messages(&envelope),
        ["cycle: a -> c -> b -> a (each is blocked by the next)"],
        "one finding naming the ids"
    );
    Ok(())
}

#[test]
fn check_exits_2_naming_each_dangling_reference() -> TestResult {
    let output = dag(&["check", "--dir", "dangling"])?;

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let envelope = envelope(&output)?;
    assert_eq!(
        finding_messages(&envelope),
        [
            "1 is blocked by 9, which is not a ticket",
            "2 is blocked by 7, which is not a ticket",
        ],
        "each (ticket, missing) pair"
    );
    Ok(())
}

#[test]
fn check_exits_0_on_the_empty_fixture() -> TestResult {
    let output = dag(&["check", "--dir", "empty"])?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        envelope(&output)?["data"]["tickets"],
        json!(0),
        "no tickets"
    );
    Ok(())
}

#[test]
fn frontier_lists_the_ready_tickets_in_a_stable_order() -> TestResult {
    let first = dag(&["frontier", "--dir", "acyclic"])?;
    let second = dag(&["frontier", "--dir", "acyclic"])?;

    assert_eq!(first.status.code(), Some(0), "{first:?}");
    assert_eq!(first.stdout, second.stdout, "byte-identical across runs");
    let envelope = envelope(&first)?;
    let ids: Vec<&str> = envelope["data"]["tickets"]
        .as_array()
        .map(|t| t.iter().filter_map(|t| t["id"].as_str()).collect())
        .unwrap_or_default();
    assert_eq!(
        ids,
        ["002-cli", "003-guard"],
        "001 is done; 004 waits for both"
    );
    assert_eq!(
        envelope["data"]["budget"],
        json!(100_000),
        "the default budget"
    );
    Ok(())
}

#[test]
fn frontier_flags_tickets_over_the_budget() -> TestResult {
    let output = dag(&["frontier", "--dir", "acyclic", "--budget", "100"])?;

    assert_eq!(
        output.status.code(),
        Some(0),
        "a flag does not fail the run"
    );
    let envelope = envelope(&output)?;
    assert_eq!(
        envelope["data"]["over_budget"],
        json!(1),
        "002-cli names a file"
    );
    assert_eq!(
        finding_messages(&envelope),
        ["002-cli is estimated at 2010 tokens, over the 100-token budget"],
        "body 10 + the unsized file 2000"
    );
    Ok(())
}

#[test]
fn flags_of_the_other_tracker_are_usage_errors() -> TestResult {
    let cases = [
        (
            vec!["check", "--repo", "octo/tickets"],
            "ERROR --repo and --issue need --tracker github\n",
        ),
        (
            vec!["check", "--tracker", "github"],
            "ERROR --tracker github needs --repo owner/name\n",
        ),
        (
            vec![
                "frontier",
                "--tracker",
                "github",
                "--repo",
                "o/r",
                "--dir",
                "x",
            ],
            "ERROR --dir needs --tracker markdown\n",
        ),
    ];
    for (args, stderr) in cases {
        let output = dag(&args)?;
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{args:?}: nothing on stdout");
        assert_eq!(String::from_utf8(output.stderr)?, stderr, "{args:?}");
    }
    Ok(())
}

#[test]
fn a_missing_tickets_directory_exits_70_with_one_error_line() -> TestResult {
    let output = dag(&["check", "--dir", "absent"])?;

    assert_eq!(output.status.code(), Some(70), "{output:?}");
    assert!(output.stdout.is_empty(), "no envelope");
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        stderr.starts_with("ERROR dag: reading the markdown tickets: listing tickets in ")
            && stderr.lines().count() == 1,
        "{stderr:?}"
    );
    Ok(())
}
