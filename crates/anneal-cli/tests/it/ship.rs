//! `anneal ship skip-rule` through the built binary, on scratch pull-request repositories.

use serde_json::json;

use crate::pr_repo::{PrRepo, TestResult};

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_small_change_to_ordinary_files_may_skip_full_review() -> TestResult {
    let repo = PrRepo::new()?;

    let envelope = repo.envelope(&["ship", "skip-rule"], 0)?;

    assert_eq!(
        (
            &envelope["data"]["skip"],
            &envelope["data"]["files"],
            &envelope["data"]["lines"],
            &envelope["data"]["base"],
        ),
        (&json!(true), &json!(1), &json!(3), &json!("main")),
        "{envelope}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn three_files_need_a_full_review() -> TestResult {
    let repo = PrRepo::new()?;
    repo.write("src/a.rs", "a\n")?;
    repo.write("src/b.rs", "b\n")?;
    repo.commit("two more files")?;

    let envelope = repo.envelope(&["ship", "skip-rule"], 1)?;

    assert_eq!(
        &envelope["data"]["reasons"],
        &json!([{"reason": "too_many_files", "files": 3}]),
        "{envelope}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_sensitive_path_needs_a_full_review() -> TestResult {
    let repo = PrRepo::new()?;
    repo.write("db/migrations/0001.sql", "create table t (id int);\n")?;
    repo.commit("a migration")?;

    let envelope = repo.envelope(&["ship", "skip-rule"], 1)?;

    assert_eq!(
        &envelope["data"]["reasons"],
        &json!([{
            "reason": "sensitive", "path": "db/migrations/0001.sql",
            "glob": "**/migrations/**", "category": "migrations",
        }]),
        "{envelope}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_glob_configured_in_the_repository_adds_a_sensitive_path() -> TestResult {
    let repo = PrRepo::new()?;
    repo.write(
        ".anneal/config.json",
        r#"{"ship": {"sensitive_globs": ["src/lib.rs"]}}"#,
    )?;

    let envelope = repo.envelope(&["ship", "skip-rule"], 1)?;

    assert_eq!(
        &envelope["data"]["reasons"][0]["category"],
        &json!("configured"),
        "{envelope}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_base_that_names_no_commit_is_a_usage_error() -> TestResult {
    let repo = PrRepo::new()?;

    let output = repo.anneal(&["ship", "skip-rule", "--base", "no-such-base"])?;

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "nothing on stdout: {output:?}");
    assert!(
        String::from_utf8(output.stderr)?.starts_with("ERROR base no-such-base"),
        "one ERROR line naming the base"
    );
    Ok(())
}
