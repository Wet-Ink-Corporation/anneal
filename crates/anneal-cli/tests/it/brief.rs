//! `anneal brief build` and `anneal lint brief` through the built binary, on a scratch
//! repository with a `feature` branch on top of `main`.

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::pr_repo::{PrRepo, TestResult};

/// Every field as `--field key=text`.
const ALL_FIELDS: [&str; 9] = [
    "goal=Add answer().",
    "repository=this repository, on main",
    "owned-scope=src/lib.rs",
    "do-not-touch=README.md",
    "invariants=answer() is pure.",
    "success-criteria=answer() returns 42.",
    "verification=cargo test",
    "edits-allowed=yes",
    "return-contract=Files changed and the test result.",
];

/// `brief build --ticket 36 --base main` with every field given, plus `extra`.
fn build_args(extra: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = ["brief", "build", "--ticket", "36", "--base", "main"]
        .map(str::to_owned)
        .to_vec();
    for field in ALL_FIELDS {
        args.extend(["--field".to_owned(), field.to_owned()]);
    }
    args.extend(extra.iter().map(|arg| (*arg).to_owned()));
    args
}

fn briefs(repo: &PrRepo) -> PathBuf {
    repo.path().join(".anneal").join("briefs")
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn build_writes_a_brief_that_lints_clean_and_a_review_package_with_the_diff() -> TestResult {
    let repo = PrRepo::new()?;
    let head = repo.head()?;
    let args = build_args(&[]);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    let envelope = repo.envelope(&args, 0)?;

    let data = &envelope["data"];
    assert_eq!(
        (&data["brief"], &data["review_package"], &data["head"]),
        (
            &json!(".anneal/briefs/36.md"),
            &json!(".anneal/briefs/36.review.md"),
            &json!(head),
        ),
        "{envelope}"
    );
    let package = fs::read_to_string(briefs(&repo).join("36.review.md"))?;
    assert!(
        package.contains("+++ b/src/lib.rs") && package.contains("answer() returns 42."),
        "the diff and the contract: {package}"
    );
    let lint = repo.envelope(&["lint", "brief", ".anneal/briefs/36.md"], 0)?;
    assert_eq!(lint["data"]["findings"], json!([]), "{lint}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_brief_built_without_fields_fails_lint_brief_naming_each_empty_field() -> TestResult {
    let repo = PrRepo::new()?;

    let built = repo.envelope(&["brief", "build", "--ticket", "36", "--base", "main"], 0)?;
    let lint = repo.envelope(&["lint", "brief", ".anneal/briefs/36.md"], 1)?;

    let empty: Vec<&Value> = lint["data"]["findings"]
        .as_array()
        .ok_or("findings")?
        .iter()
        .map(|finding| &finding["field"])
        .collect();
    assert_eq!(
        empty,
        [
            &json!("Goal"),
            &json!("Owned scope"),
            &json!("Do not touch"),
            &json!("Invariants"),
            &json!("Success criteria"),
            &json!("Verification"),
            &json!("Edits allowed"),
        ],
        "repository and return contract have defaults: {lint}"
    );
    assert_eq!(
        built["data"]["empty_fields"].as_array().map(Vec::len),
        Some(7),
        "build lists them too: {built}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_subagent_that_waits_on_ci_gets_cache_ttl_in_the_volatile_section() -> TestResult {
    let repo = PrRepo::new()?;
    let args = build_args(&["--waits-on-ci"]);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    repo.envelope(&args, 0)?;

    let brief = fs::read_to_string(briefs(&repo).join("36.md"))?;
    let volatile = brief.find("## Volatile").ok_or("a volatile section")?;
    let ttl = brief.find("- cacheTtl: 1h").ok_or("cacheTtl")?;
    assert!(ttl > volatile, "in the trailing volatile section: {brief}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_base_that_is_not_an_ancestor_of_head_is_a_bad_range_and_writes_nothing() -> TestResult {
    let repo = PrRepo::new()?;
    repo.git(&["checkout", "-q", "-b", "sibling", "main"])?;
    repo.write("docs/other.md", "elsewhere\n")?;
    repo.commit("sibling")?;
    repo.git(&["checkout", "-q", "feature"])?;

    let envelope = repo.envelope(
        &["brief", "build", "--ticket", "36", "--base", "sibling"],
        2,
    )?;

    assert_eq!(
        envelope["findings"][0]["key"],
        json!("brief/range"),
        "{envelope}"
    );
    assert!(!briefs(&repo).exists(), "nothing written");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_base_that_names_no_commit_is_a_bad_range() -> TestResult {
    let repo = PrRepo::new()?;

    let envelope = repo.envelope(
        &["brief", "build", "--ticket", "36", "--base", "no-such-ref"],
        2,
    )?;

    assert_eq!(
        (&envelope["ok"], &envelope["findings"][0]["key"]),
        (&json!(false), &json!("brief/range")),
        "{envelope}"
    );
    assert!(!briefs(&repo).exists(), "nothing written");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn when_the_review_package_cannot_be_written_no_brief_is_written_either() -> TestResult {
    let repo = PrRepo::new()?;
    let blocker = briefs(&repo).join("36.review.md");
    fs::create_dir_all(&blocker)?;
    fs::write(blocker.join("keep"), "a directory where the package goes\n")?;
    let args = build_args(&[]);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    let output = repo.anneal(&args)?;

    assert_eq!(output.status.code(), Some(70), "{output:?}");
    let mut left: Vec<String> = fs::read_dir(briefs(&repo))?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_, _>>()?;
    left.sort();
    assert_eq!(
        left,
        ["36.review.md"],
        "no brief without its package, and no temporary file left"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn an_unknown_field_key_is_a_usage_error() -> TestResult {
    let repo = PrRepo::new()?;

    let output = repo.anneal(&[
        "brief",
        "build",
        "--ticket",
        "36",
        "--base",
        "main",
        "--field",
        "colour=blue",
    ])?;

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "no envelope for a usage error");
    Ok(())
}
