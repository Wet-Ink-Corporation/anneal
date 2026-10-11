//! `anneal grill frontier|confirm` through the built binary: the frontier envelope, invalid
//! trees, and the confirmation that is appended only with the human's words and no open node.

use std::error::Error;
use std::fmt::Write as _;
use std::path::Path;
use std::process::Output;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::support::anneal_in;

type TestResult = Result<(), Box<dyn Error>>;

/// A node as JSON with a recommended answer and confidence 0.5: a decision (`is_decision`
/// 0.9) when its id starts with `decide`, else a fact (0.2).
fn node(id: &str, depends_on: &[&str], status: &str) -> Value {
    let is_decision = if id.starts_with("decide") { 0.9 } else { 0.2 };
    json!({
        "id": id,
        "question": format!("question {id}?"),
        "depends_on": depends_on,
        "status": status,
        "answer": format!("answer {id}"),
        "is_decision": is_decision,
        "confidence": 0.5,
    })
}

/// Writes a version-1 design tree with `nodes` and turn confidences `turns` to
/// `<dir>/tree.json` and returns its bytes.
fn write_tree(dir: &Path, nodes: &[Value], turns: &[f64]) -> Result<Vec<u8>, Box<dyn Error>> {
    let turns: Vec<Value> = turns.iter().map(|c| json!({"confidence": c})).collect();
    let bytes = serde_json::to_vec_pretty(&json!({
        "version": 1,
        "goal": "export orders as CSV",
        "turns": turns,
        "nodes": nodes,
    }))?;
    std::fs::write(dir.join("tree.json"), &bytes)?;
    Ok(bytes)
}

/// Runs `anneal grill <args>` in `cwd`.
fn grill(cwd: &Path, args: &[&str]) -> Result<Output, Box<dyn Error>> {
    Ok(anneal_in(cwd).arg("grill").args(args).output()?)
}

/// The JSON envelope on stdout, after checking the exit code and that stderr is empty.
fn envelope(output: &Output, code: i32) -> Result<Value, Box<dyn Error>> {
    assert_eq!(output.status.code(), Some(code), "{output:?}");
    assert!(output.stderr.is_empty(), "nothing on stderr: {output:?}");
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn sha256_hex(bytes: &[u8]) -> Result<String, Box<dyn Error>> {
    let mut hex = String::from("sha256:");
    for byte in Sha256::digest(bytes) {
        write!(hex, "{byte:02x}")?;
    }
    Ok(hex)
}

fn finding_keys(envelope: &Value) -> Vec<&str> {
    envelope["findings"]
        .as_array()
        .map(|findings| {
            findings
                .iter()
                .filter_map(|finding| finding["key"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn frontier_lists_the_unblocked_open_questions_and_the_restate_gate() -> TestResult {
    let dir = tempfile::tempdir()?;
    let bytes = write_tree(
        dir.path(),
        &[
            node("scope", &[], "answered"),
            node("decide-format", &["scope"], "open"),
            node("columns", &["decide-format"], "open"),
            node("audience", &[], "open"),
            node("later", &[], "deferred"),
        ],
        &[0.4, 0.6, 0.85],
    )?;

    let result = envelope(&grill(dir.path(), &["frontier", "--tree", "tree.json"])?, 0)?;

    assert_eq!(result["command"], "grill frontier", "{result}");
    let data = &result["data"];
    assert_eq!(
        data["frontier"],
        json!([
            node("audience", &[], "open"),
            node("decide-format", &["scope"], "open")
        ]),
        "open nodes with every dependency answered, by id, as written"
    );
    assert_eq!(
        (&data["ask"], &data["look_up"]),
        (&json!(["decide-format"]), &json!(["audience"])),
        "decisions are asked, facts looked up"
    );
    assert_eq!(
        (
            &data["nodes"],
            &data["open"],
            &data["answered"],
            &data["deferred"]
        ),
        (&json!(5), &json!(3), &json!(1), &json!(1)),
        "counts by status"
    );
    assert_eq!(
        data["confidence"],
        json!({
            "turns": 3, "latest": 0.85, "best": 0.85, "turns_without_gain": 0, "restate": true,
            "restate_at": 0.8, "restate_after_turns_without_gain": 3,
        }),
        "0.85 reached the restate gate"
    );
    assert_eq!(data["hash"], json!(sha256_hex(&bytes)?), "the file's hash");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn an_invalid_tree_exits_1_with_one_finding_per_problem_and_confirms_nothing() -> TestResult {
    let dir = tempfile::tempdir()?;
    write_tree(
        dir.path(),
        &[node("a", &["b"], "open"), node("b", &["a"], "open")],
        &[],
    )?;
    let store = dir.path().join("ledger.sqlite3");

    let frontier = envelope(&grill(dir.path(), &["frontier", "--tree", "tree.json"])?, 1)?;
    let confirm = envelope(
        &grill(
            dir.path(),
            &[
                "confirm",
                "--tree",
                "tree.json",
                "--quote",
                "yes",
                "--store",
                "ledger.sqlite3",
            ],
        )?,
        1,
    )?;

    for result in [&frontier, &confirm] {
        assert_eq!(result["ok"], false, "{result}");
        assert_eq!(finding_keys(result), ["grill/cycle:a->b->a"], "{result}");
    }
    assert!(!store.exists(), "no ledger was opened");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn confirm_without_the_humans_words_is_a_usage_error_and_records_nothing() -> TestResult {
    let dir = tempfile::tempdir()?;
    write_tree(dir.path(), &[node("a", &[], "answered")], &[0.9])?;
    let store = dir.path().join("ledger.sqlite3");
    let base = [
        "confirm",
        "--tree",
        "tree.json",
        "--store",
        "ledger.sqlite3",
    ];

    for quote in [None, Some(""), Some("  \t")] {
        let mut args = base.to_vec();
        if let Some(words) = quote {
            args.extend(["--quote", words]);
        }

        let output = grill(dir.path(), &args)?;

        assert_eq!(output.status.code(), Some(2), "{quote:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{quote:?}: nothing on stdout");
        let stderr = String::from_utf8(output.stderr)?;
        assert!(
            stderr.starts_with("ERROR ") && stderr.lines().count() == 1,
            "{quote:?}: one ERROR line: {stderr}"
        );
        assert!(
            stderr.contains("--quote"),
            "{quote:?}: the line asks for --quote: {stderr}"
        );
    }
    assert!(!store.exists(), "no ledger was opened");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn confirm_with_an_open_question_exits_3_and_records_nothing() -> TestResult {
    let dir = tempfile::tempdir()?;
    write_tree(
        dir.path(),
        &[node("a", &[], "answered"), node("b", &["a"], "open")],
        &[0.9],
    )?;
    let store = dir.path().join("ledger.sqlite3");

    let result = envelope(
        &grill(
            dir.path(),
            &[
                "confirm",
                "--tree",
                "tree.json",
                "--quote",
                "yes",
                "--store",
                "ledger.sqlite3",
            ],
        )?,
        3,
    )?;

    assert_eq!(finding_keys(&result), ["grill/open:b"], "{result}");
    assert!(!store.exists(), "no ledger was opened");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn confirm_appends_one_grill_confirmed_with_the_tree_hash_and_the_quote() -> TestResult {
    let dir = tempfile::tempdir()?;
    let bytes = write_tree(
        dir.path(),
        &[node("a", &[], "answered"), node("b", &["a"], "deferred")],
        &[0.5, 0.82],
    )?;
    let quote = "Yes, that's it. Go ahead.";

    let result = envelope(
        &grill(
            dir.path(),
            &[
                "confirm",
                "--tree",
                "tree.json",
                "--quote",
                quote,
                "--session",
                "s1",
                "--store",
                "ledger.sqlite3",
            ],
        )?,
        0,
    )?;
    let ledger = anneal_in(dir.path())
        .args(["ledger", "--store", "ledger.sqlite3", "query"])
        .output()?;
    let ledger = envelope(&ledger, 0)?;

    assert_eq!(result["command"], "grill confirm", "{result}");
    let events = ledger["data"]["events"].as_array().ok_or("events")?;
    assert_eq!(events.len(), 1, "exactly one event: {ledger}");
    let event = &events.first().ok_or("one event")?["event"];
    assert_eq!(
        (
            &event["type"],
            &event["plan"],
            &event["hash"],
            &event["quote"],
            &event["session_id"]
        ),
        (
            &json!("grill_confirmed"),
            &json!("tree.json"),
            &json!(sha256_hex(&bytes)?),
            &json!(quote),
            &json!("s1")
        ),
        "the tree's path and hash, the human's words verbatim, the session"
    );
    assert_eq!(
        result["data"]["event"], *event,
        "the envelope shows the event"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_missing_tree_file_is_a_usage_error() -> TestResult {
    let dir = tempfile::tempdir()?;

    let output = grill(dir.path(), &["frontier", "--tree", "absent.json"])?;

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "nothing on stdout");
    assert_eq!(
        String::from_utf8(output.stderr)?,
        "ERROR design tree absent.json does not exist\n",
        "one ERROR line naming the file"
    );
    Ok(())
}

/// Asserts a usage error: exit 2, nothing on stdout, one `ERROR` line containing `names`.
fn assert_usage_error(output: &Output, names: &str) -> TestResult {
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "nothing on stdout: {output:?}");
    let stderr = std::str::from_utf8(&output.stderr)?;
    assert!(
        stderr.starts_with("ERROR ") && stderr.lines().count() == 1 && stderr.contains(names),
        "one ERROR line naming {names:?}: {stderr}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn confirm_with_a_tree_path_too_long_for_a_plan_tag_is_a_usage_error_and_opens_no_ledger()
-> TestResult {
    let dir = tempfile::tempdir()?;
    let nested = ["a", "b", "c"].map(|letter| letter.repeat(100)).join("/");
    std::fs::create_dir_all(dir.path().join(&nested))?;
    write_tree(
        &dir.path().join(&nested),
        &[node("a", &[], "answered")],
        &[0.9],
    )?;
    let tree = format!("{nested}/tree.json");
    let store = dir.path().join("ledger.sqlite3");

    let output = grill(
        dir.path(),
        &[
            "confirm",
            "--tree",
            &tree,
            "--quote",
            "yes",
            "--store",
            "ledger.sqlite3",
        ],
    )?;

    assert_usage_error(&output, "plan")?;
    assert!(!store.exists(), "no ledger was opened");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_directory_as_the_tree_is_a_usage_error() -> TestResult {
    let dir = tempfile::tempdir()?;
    std::fs::create_dir(dir.path().join("trees"))?;

    let output = grill(dir.path(), &["frontier", "--tree", "trees"])?;

    assert_usage_error(&output, "trees")
}
