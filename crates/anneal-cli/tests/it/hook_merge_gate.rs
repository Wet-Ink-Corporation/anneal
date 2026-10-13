//! `anneal hook merge-gate` through the built binary, on scratch pull-request repositories:
//! denied with no verdict and after a real code change, allowed after a rebase-only change,
//! the review requirement and the skip-rule, a human approval counting only while
//! `review.require_plugin` is false, silence for other calls, and failing closed.

use std::process::Output;

use serde_json::Value;

use crate::pr_repo::{ORIGIN_REPO, PrRepo, TestResult};

/// The deny decision's reason, or an error when `output` is not one deny decision.
fn deny_reason(output: &Output) -> TestResult<String> {
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let decision: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        decision.pointer("/hookSpecificOutput/permissionDecision"),
        Some(&Value::from("deny")),
        "{decision}"
    );
    let reason = decision
        .pointer("/hookSpecificOutput/permissionDecisionReason")
        .and_then(Value::as_str)
        .ok_or("a reason")?;
    assert!(reason.starts_with("ERROR "), "one ERROR line: {reason}");
    assert!(!reason.contains('\n'), "one line: {reason}");
    Ok(reason.to_owned())
}

/// Asserts the hook let the call through: exit 0 and no output.
fn assert_allowed(output: &Output) {
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stdout.is_empty(), "silent: {output:?}");
    assert!(output.stderr.is_empty(), "silent: {output:?}");
}

/// The `gate_blocked` points in the ledger.
fn blocked_points(repo: &PrRepo) -> TestResult<Vec<String>> {
    let envelope = repo.envelope(&["ledger", "query", "--type", "gate_blocked"], 0)?;
    let events = envelope
        .pointer("/data/events")
        .and_then(Value::as_array)
        .ok_or("an event list")?;
    Ok(events
        .iter()
        .filter_map(|event| event.pointer("/event/point").and_then(Value::as_str))
        .map(str::to_owned)
        .collect())
}

/// Appends a review plugin's `verdict` for pull request 7 at `HEAD`, as the plugin would
/// through `anneal ledger append` (`verdict set` records only levels).
fn append_verdict(repo: &PrRepo, verdict: &str) -> TestResult {
    let event = serde_json::json!({
        "type": "verdict_set", "pr": 7, "verdict": verdict, "source": "review_plugin",
        "sha": repo.head()?, "actor": "review-plugin", "ts": "2026-10-12T09:00:00Z",
    })
    .to_string();
    repo.envelope(&["ledger", "append", &event], 0)?;
    Ok(())
}

/// Adds three files, so the change is too big to skip review.
fn widen(repo: &PrRepo) -> TestResult {
    for name in ["src/a.rs", "src/b.rs", "src/c.rs"] {
        repo.write(name, "pub fn f() {}\n")?;
    }
    repo.commit("three more files")?;
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn merge_is_denied_with_no_verdict_and_no_ledger_is_created() -> TestResult {
    let repo = PrRepo::new()?;

    let output = repo.merge_pinned("gh pr merge 7 --squash")?;

    let reason = deny_reason(&output)?;
    assert!(
        reason.contains("#7") && reason.contains("anneal verdict set"),
        "names the pull request and the command: {reason}"
    );
    assert!(!repo.ledger().exists(), "the gate never creates a ledger");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn merge_of_a_pull_request_without_verdicts_is_denied_and_recorded() -> TestResult {
    let repo = PrRepo::new()?;
    repo.set_verdict("tests-verified", "local")?;

    let output = repo.merge_pinned("gh pr merge 9")?;

    let reason = deny_reason(&output)?;
    assert!(reason.contains("#9"), "{reason}");
    assert_eq!(blocked_points(&repo)?, ["merge-gate.no-verdict"]);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn merge_is_denied_with_a_stale_verdict_after_a_real_code_change() -> TestResult {
    let repo = PrRepo::new()?;
    repo.set_verdict("tests-verified", "local")?;
    repo.change_code()?;

    let output = repo.merge_pinned("gh pr merge 7")?;

    let reason = deny_reason(&output)?;
    assert!(
        reason.contains("stale") || reason.contains("other code"),
        "{reason}"
    );
    assert_eq!(blocked_points(&repo)?, ["merge-gate.stale"]);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn merge_is_allowed_after_a_rebase_only_change() -> TestResult {
    let repo = PrRepo::new()?;
    repo.set_verdict("tests-verified", "local")?;
    repo.rebase()?;

    let output = repo.merge_pinned("gh pr merge 7 --squash --delete-branch")?;

    assert_allowed(&output);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_change_the_skip_rule_does_not_skip_needs_a_review_verdict() -> TestResult {
    let repo = PrRepo::new()?;
    widen(&repo)?;
    repo.set_verdict("tests-verified", "local")?;

    let output = repo.merge_pinned("gh pr merge 7")?;

    let reason = deny_reason(&output)?;
    assert!(reason.contains("review"), "{reason}");
    assert_eq!(blocked_points(&repo)?, ["merge-gate.review"]);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_human_approval_counts_as_the_review_only_while_require_plugin_is_false() -> TestResult {
    let repo = PrRepo::new()?;
    widen(&repo)?;
    repo.set_verdict("tests-verified", "local")?;
    repo.set_verdict("approved", "human")?;

    assert_allowed(&repo.merge_pinned("gh pr merge 7")?);

    repo.write(
        ".anneal/config.json",
        r#"{"review": {"require_plugin": true}}"#,
    )?;
    let reason = deny_reason(&repo.merge_pinned("gh pr merge 7")?)?;
    assert!(reason.contains("review-plugin"), "{reason}");

    repo.set_verdict("reviewed", "review-plugin")?;
    assert_allowed(&repo.merge_pinned("gh pr merge 7")?);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_review_without_a_tests_verified_verdict_is_not_enough() -> TestResult {
    let repo = PrRepo::new()?;
    append_verdict(&repo, "noise_only")?;

    let reason = deny_reason(&repo.merge_pinned("gh pr merge 7")?)?;

    assert!(reason.contains("tests-verified"), "{reason}");
    assert_eq!(blocked_points(&repo)?, ["merge-gate.tests"]);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_later_changes_requested_verdict_voids_the_tests_verified_one() -> TestResult {
    let repo = PrRepo::new()?;
    repo.set_verdict("tests-verified", "local")?;
    append_verdict(&repo, "changes_requested")?;

    let reason = deny_reason(&repo.merge_pinned("gh pr merge 7")?)?;

    assert!(reason.contains("requests changes"), "{reason}");
    assert_eq!(blocked_points(&repo)?, ["merge-gate.changes-requested"]);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn match_head_commit_names_the_head_that_is_merged() -> TestResult {
    let repo = PrRepo::new()?;
    repo.ensure_origin()?;
    let verified = repo.head()?;
    repo.set_verdict("tests-verified", "local")?;
    repo.change_code()?;

    let pinned = format!("gh pr merge 7 --match-head-commit {verified} --repo {ORIGIN_REPO}");
    assert_allowed(&repo.merge_gate(&pinned)?);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_merge_that_names_no_pull_request_number_is_denied() -> TestResult {
    let repo = PrRepo::new()?;
    repo.set_verdict("tests-verified", "local")?;

    for command in [
        "gh pr merge",
        "gh pr merge feature",
        "gh pr merge 7 --repo other/repo",
    ] {
        let reason = deny_reason(&repo.merge_gate(command)?)?;
        assert!(reason.contains("merge-gate"), "{command}: {reason}");
    }
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn other_commands_and_tools_are_silent() -> TestResult {
    let repo = PrRepo::new()?;

    for command in ["git status", "gh pr view 7", "gh pr merge 7 --disable-auto"] {
        assert_allowed(&repo.merge_gate(command)?);
    }
    let edit = br#"{"tool_name": "Edit", "tool_input": {"file_path": "x"}}"#;
    assert_allowed(&repo.hook_stdin(edit)?);
    assert!(!repo.ledger().exists(), "nothing created");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn unreadable_input_and_commands_are_denied() -> TestResult {
    let repo = PrRepo::new()?;

    let reason = deny_reason(&repo.hook_stdin(b"not json")?)?;
    assert!(reason.contains("input"), "{reason}");
    let reason = deny_reason(&repo.merge_gate("gh pr merge 7 'unclosed")?)?;
    assert!(reason.contains("merge-gate"), "{reason}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_merge_without_a_full_head_commit_pin_is_denied_naming_the_command() -> TestResult {
    let repo = PrRepo::new()?;
    repo.set_verdict("tests-verified", "local")?;
    let head = repo.head()?;
    let short = head.get(..7).ok_or("a full id")?;
    let command = format!("gh pr merge 7 --match-head-commit {head}");

    for unpinned in [
        "gh pr merge 7".to_owned(),
        format!("gh pr merge 7 --match-head-commit {short}"),
    ] {
        let reason = deny_reason(&repo.merge_gate(&unpinned)?)?;
        assert!(reason.contains(&command), "{unpinned}: {reason}");
    }
    assert_eq!(
        blocked_points(&repo)?,
        ["merge-gate.head", "merge-gate.head"]
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_pull_request_url_counts_only_for_the_origin_repository() -> TestResult {
    let repo = PrRepo::new()?;
    repo.git(&["remote", "add", "origin", "git@github.com:Owner/Repo.git"])?;
    repo.set_verdict("tests-verified", "local")?;

    let ours = repo.merge_pinned("gh pr merge https://github.com/owner/repo/pull/7")?;
    let theirs = repo.merge_pinned("gh pr merge https://github.com/other/repo/pull/7")?;

    assert_allowed(&ours);
    let reason = deny_reason(&theirs)?;
    assert!(reason.contains("other/repo"), "{reason}");
    assert_eq!(blocked_points(&repo)?, ["merge-gate.repo"]);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_gh_repository_or_host_set_in_the_command_is_denied() -> TestResult {
    let repo = PrRepo::new()?;
    repo.set_verdict("tests-verified", "local")?;

    for command in [
        "GH_REPO=o/x gh pr merge 7",
        "env GH_HOST=ghe.example gh pr merge 7",
    ] {
        let reason = deny_reason(&repo.merge_pinned(command)?)?;
        assert!(reason.contains("GH_"), "{command}: {reason}");
    }
    assert_eq!(blocked_points(&repo)?, ["merge-gate.env", "merge-gate.env"]);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn gh_repository_variables_set_through_brace_expansion_are_denied() -> TestResult {
    let repo = PrRepo::new()?;
    repo.set_verdict("tests-verified", "local")?;

    let reason = deny_reason(&repo.merge_pinned("export GH_RE{PO,X}=o/x; gh pr merge 7")?)?;

    assert!(reason.contains("sets GH_REPO"), "{reason}");
    assert_eq!(blocked_points(&repo)?, ["merge-gate.env"]);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_merge_after_a_directory_change_is_denied() -> TestResult {
    let repo = PrRepo::new()?;
    repo.set_verdict("tests-verified", "local")?;

    for command in ["cd /tmp && gh pr merge 7", "env -C /tmp gh pr merge 7"] {
        let reason = deny_reason(&repo.merge_pinned(command)?)?;
        assert!(reason.contains("before gh pr merge"), "{command}: {reason}");
    }
    assert_eq!(
        blocked_points(&repo)?,
        ["merge-gate.directory", "merge-gate.directory"]
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_merge_naming_the_origin_repository_in_every_form_is_allowed() -> TestResult {
    let repo = PrRepo::new()?;
    repo.ensure_origin()?;
    repo.set_verdict("tests-verified", "local")?;
    let head = repo.head()?;

    for named in [
        format!("--repo {ORIGIN_REPO}"),
        format!("--repo={ORIGIN_REPO}"),
        format!("-R {ORIGIN_REPO}"),
        format!("-R{ORIGIN_REPO}"),
        "-R GitHub.com/Owner/Repo".to_owned(),
        format!("--repo github.com/other/repo --repo {ORIGIN_REPO}"),
    ] {
        let command = format!("gh pr merge 7 --match-head-commit {head} {named}");
        assert_allowed(&repo.merge_gate(&command)?);
    }
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_merge_without_the_origin_repository_named_is_denied_naming_the_command() -> TestResult {
    let repo = PrRepo::new()?;
    repo.ensure_origin()?;
    repo.set_verdict("tests-verified", "local")?;
    let head = repo.head()?;
    let exact = format!("`gh pr merge 7 --match-head-commit {head} --repo {ORIGIN_REPO}`");

    let rows = [
        ("", "names no --repo"),
        (
            "--repo github.com/other/repo",
            "names github.com/other/repo",
        ),
        (
            "--repo=ghe.example/owner/repo",
            "names ghe.example/owner/repo",
        ),
        ("-R github.com/owner/other", "names github.com/owner/other"),
        (
            "--repo github.com/owner/repo --repo github.com/other/repo",
            "names github.com/other/repo",
        ),
    ];
    for (named, why) in rows {
        let command = format!("gh pr merge 7 --match-head-commit {head} {named}");
        let reason = deny_reason(&repo.merge_gate(&command)?)?;
        assert!(
            reason.contains(why) && reason.contains(&exact),
            "{command}: {reason}"
        );
    }
    let short = format!("gh pr merge 7 --match-head-commit {head} --repo owner/repo");
    let reason = deny_reason(&repo.merge_gate(&short)?)?;
    assert!(reason.contains("HOST/OWNER/REPO"), "{reason}");
    assert_eq!(blocked_points(&repo)?, ["merge-gate.repo"; 6]);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_merge_in_a_repository_without_an_origin_is_denied() -> TestResult {
    let repo = PrRepo::new()?;
    repo.set_verdict("tests-verified", "local")?;
    let head = repo.head()?;

    let command = format!("gh pr merge 7 --match-head-commit {head} --repo {ORIGIN_REPO}");
    let reason = deny_reason(&repo.merge_gate(&command)?)?;

    assert!(reason.contains("origin remote"), "{reason}");
    assert_eq!(blocked_points(&repo)?, ["merge-gate.repo"]);
    Ok(())
}

/// The bypasses the reviews of #89 found, around `merge`, each with the check that denies it.
fn bypass_rows(merge: &str) -> Vec<(String, &'static str)> {
    vec![
        // Brace sequences building GH_REPO or GH_HOST.
        (
            format!("export GH_REP{{O..O}}=o/x; {merge}"),
            "merge-gate.unreadable",
        ),
        (
            format!("env GH_HO{{S..S}}T=ghe.example {merge}"),
            "merge-gate.unreadable",
        ),
        // A function or trap body run after a later cd.
        (
            format!("f(){{ {merge}; }}; cd /other && f"),
            "merge-gate.directory",
        ),
        (
            format!("trap '{merge}' EXIT; cd /other"),
            "merge-gate.directory",
        ),
        // Login shells start in the user's home.
        (format!("sudo -i {merge}"), "merge-gate.directory"),
        (format!("sudo --login {merge}"), "merge-gate.directory"),
        (format!("su - -c '{merge}'"), "merge-gate.directory"),
        (format!("su -l -c '{merge}'"), "merge-gate.directory"),
        (format!("su --login -c '{merge}'"), "merge-gate.directory"),
        // env -S splits its string into env's own options, -C among them.
        (format!("env -S '-C /tmp {merge}'"), "merge-gate.directory"),
        // Left to --repo: an env short cluster, a git pager, a dynamic name.
        (format!("env -vu X -C /tmp {merge}"), "merge-gate.repo"),
        (
            format!("git -C /other -c core.pager='{merge}' log"),
            "merge-gate.repo",
        ),
        (
            format!("export $(printf GH_RE%sPO)=o/x; {merge}"),
            "merge-gate.repo",
        ),
        // Round 2: a --repo the command does not write out, or one a vanishing word shifts.
        (
            format!(r#"r={ORIGIN_REPO}; {merge} --repo "$r""#),
            "merge-gate.repo",
        ),
        (
            format!(r#"false && r={ORIGIN_REPO}; {merge} --repo "$r""#),
            "merge-gate.repo",
        ),
        (
            format!("r={ORIGIN_REPO}; unset r; {merge} --repo $r"),
            "merge-gate.repo",
        ),
        (
            format!("{merge} --body $x --repo {ORIGIN_REPO}"),
            "merge-gate.repo",
        ),
        (
            format!("{merge} $x --repo {ORIGIN_REPO}"),
            "merge-gate.repo",
        ),
    ]
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn every_bypass_from_the_review_of_89_is_denied() -> TestResult {
    let repo = PrRepo::new()?;
    repo.ensure_origin()?;
    repo.set_verdict("tests-verified", "local")?;
    let head = repo.head()?;
    let merge = format!("gh pr merge 7 --match-head-commit {head}");

    let rows = bypass_rows(&merge);
    let mut expected = Vec::new();
    for (command, point) in &rows {
        deny_reason(&repo.merge_gate(command)?)?;
        expected.push(*point);
    }

    assert_eq!(blocked_points(&repo)?, expected);
    Ok(())
}
