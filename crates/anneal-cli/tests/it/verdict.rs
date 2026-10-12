//! `anneal verdict set|valid` through the built binary, on scratch pull-request repositories.

use serde_json::{Value, json};

use crate::pr_repo::{PrRepo, TestResult};

fn valid(repo: &PrRepo, code: i32) -> TestResult<Value> {
    repo.envelope(&["verdict", "valid", "--pr", "7"], code)
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn verdict_set_records_the_level_source_commit_patch_id_and_base() -> TestResult {
    let repo = PrRepo::new()?;
    let head = repo.head()?;

    let envelope = repo.set_verdict("tests-verified", "local")?;

    let data = &envelope["data"];
    assert_eq!(
        (
            &data["type"],
            &data["pr"],
            &data["verdict"],
            &data["source"],
            &data["sha"],
            &data["base"],
        ),
        (
            &json!("verdict_set"),
            &json!(7),
            &json!("tests_verified"),
            &json!("local"),
            &json!(head),
            &json!("main"),
        ),
        "{envelope}"
    );
    let patch_id = data["patch_id"].as_str().ok_or("a patch id")?;
    assert_eq!(patch_id.len(), 40, "a SHA-1 patch id: {patch_id}");
    assert!(
        repo.ledger().is_file(),
        "written to the ledger the hook reads"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn verdict_set_refuses_a_commit_that_does_not_exist_and_records_nothing() -> TestResult {
    let repo = PrRepo::new()?;

    let output = repo.anneal(&[
        "verdict",
        "set",
        "--pr",
        "7",
        "--sha",
        "no-such-commit",
        "--level",
        "approved",
        "--source",
        "human",
    ])?;

    assert_eq!(output.status.code(), Some(2), "a usage error: {output:?}");
    assert!(output.stdout.is_empty(), "nothing on stdout");
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        stderr.starts_with("ERROR ") && stderr.contains("no-such-commit"),
        "{stderr}"
    );
    assert!(!repo.ledger().exists(), "no ledger created");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn with_no_verdict_valid_exits_1_and_creates_no_ledger() -> TestResult {
    let repo = PrRepo::new()?;

    let envelope = valid(&repo, 1)?;

    assert_eq!(
        (&envelope["ok"], &envelope["data"]["invalid"]),
        (&json!(false), &json!("no_verdict")),
        "{envelope}"
    );
    assert!(!repo.ledger().exists(), "valid never creates a ledger");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_verdict_for_the_head_is_valid() -> TestResult {
    let repo = PrRepo::new()?;
    repo.set_verdict("tests-verified", "local")?;

    let envelope = valid(&repo, 0)?;

    assert_eq!(
        (
            &envelope["data"]["verdict"]["verdict"],
            &envelope["data"]["verdict"]["fresh"],
            &envelope["data"]["tests"],
            &envelope["data"]["review"],
        ),
        (
            &json!("tests_verified"),
            &json!("by_sha"),
            &json!(true),
            &json!(false),
        ),
        "{envelope}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn after_a_rebase_only_change_the_verdict_stays_valid() -> TestResult {
    let repo = PrRepo::new()?;
    repo.set_verdict("tests-verified", "local")?;
    repo.rebase()?;

    let envelope = valid(&repo, 0)?;

    assert_eq!(
        &envelope["data"]["verdict"]["fresh"],
        &json!("by_patch_id"),
        "{envelope}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn after_a_real_code_change_the_verdict_is_stale() -> TestResult {
    let repo = PrRepo::new()?;
    repo.set_verdict("approved", "human")?;
    repo.change_code()?;

    let envelope = valid(&repo, 1)?;

    assert_eq!(
        (&envelope["data"]["invalid"], &envelope["data"]["stale"]),
        (&json!("stale"), &json!(1)),
        "{envelope}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_latest_noise_only_verdict_exits_3() -> TestResult {
    let repo = PrRepo::new()?;
    let head = repo.head()?;
    let event = json!({
        "type": "verdict_set", "pr": 7, "verdict": "noise_only", "source": "review_plugin",
        "sha": head, "actor": "review-plugin", "ts": "2026-10-12T09:00:00Z",
    })
    .to_string();
    repo.envelope(&["ledger", "append", &event], 0)?;

    let envelope = valid(&repo, 3)?;

    assert_eq!(
        (&envelope["ok"], &envelope["data"]["review"]),
        (&json!(false), &json!(true)),
        "noise-only is its own code; the review plugin's noise-only still counts as a review"
    );
    Ok(())
}
