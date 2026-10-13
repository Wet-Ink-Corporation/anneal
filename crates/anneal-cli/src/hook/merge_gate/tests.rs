use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;

use super::pull::{Pull, origin_repo, pull};
use super::{Check, run};
use crate::hook::HookEnv;
use crate::hook::guard::{PrMerge, Target};

fn merge(target: Target, repo: Option<&str>, match_head: Option<&str>) -> PrMerge {
    PrMerge {
        target,
        repo: repo.map(str::to_owned),
        match_head: match_head.map(str::to_owned),
        url_repo: None,
        env: None,
        moved: None,
    }
}

fn no_cwd() -> io::Result<PathBuf> {
    Err(io::Error::other("the test gives no working directory"))
}

/// Runs the gate on `stdin`, with no working directory and no environment.
fn gate(stdin: &[u8]) -> String {
    let mut out = Vec::new();
    let env = HookEnv {
        stdin,
        current_dir: no_cwd,
        lookup: |_: &str| None,
    };
    run(env, &mut out).expect("a Vec accepts every write");
    String::from_utf8(out).expect("UTF-8")
}

#[test]
fn a_numbered_merge_gives_its_number_and_pinned_head() {
    let pinned = merge(Target::Number(7), None, Some("abc"));

    assert_eq!(
        pull(&pinned),
        Ok(Pull {
            pr: 7,
            pin: Some("abc"),
            url_repo: None
        })
    );
}

#[test]
fn a_merge_whose_command_sets_gh_repo_or_host_is_denied() {
    let mut set = merge(Target::Number(7), None, None);
    set.env = Some("GH_REPO".to_owned());

    let denial = pull(&set).expect_err("denied");

    assert_eq!(denial.check, Check::Env, "{denial:?}");
}

#[test]
fn a_merge_run_after_a_directory_change_is_denied_naming_the_change() {
    let mut moved = merge(Target::Number(7), None, None);
    moved.moved = Some("cd");

    let denial = pull(&moved).expect_err("denied");

    assert_eq!(
        (denial.check, denial.reason.as_str()),
        (
            Check::Directory,
            "merge-gate: the command runs cd before gh pr merge, so gh may read another \
             repository; run gh pr merge from this worktree without changing directory first"
        ),
        "{denial:?}"
    );
}

#[test]
fn an_origin_url_reads_as_host_owner_and_repository() {
    let rows = [
        (
            "git@github.com:Owner/Repo.git\n",
            Some("github.com/owner/repo"),
        ),
        ("https://github.com/o/r", Some("github.com/o/r")),
        (
            "https://user@github.com:443/o/r.git/",
            Some("github.com/o/r"),
        ),
        ("ssh://git@ghe.example:22/o/r.git", Some("ghe.example/o/r")),
        ("/srv/git/r.git", None),
        ("https://github.com/o", None),
        ("https://github.com/o/r/extra", None),
    ];

    for (url, expected) in rows {
        assert_eq!(origin_repo(url).as_deref(), expected, "{url}");
    }
}

#[test]
fn a_merge_without_a_number_or_in_another_repository_is_denied_with_its_check() {
    let cases = [
        (merge(Target::Missing, None, None), Check::Target),
        (merge(Target::Unnumbered, None, None), Check::Target),
        (merge(Target::Number(7), Some("o/r"), None), Check::Repo),
    ];

    for (merge, check) in cases {
        let denial = pull(&merge).expect_err("denied");
        assert_eq!(denial.check, check, "{merge:?}");
        assert!(denial.reason.starts_with("merge-gate: "), "{denial:?}");
    }
}

#[test]
fn every_check_has_its_own_id() {
    let checks = [
        Check::BadInput,
        Check::Unreadable,
        Check::Target,
        Check::Repo,
        Check::Env,
        Check::Directory,
        Check::Head,
        Check::NoVerdict,
        Check::Stale,
        Check::ChangesRequested,
        Check::Tests,
        Check::Review,
        Check::Unjudged,
    ];

    let ids: BTreeSet<&str> = checks.iter().map(|check| check.id()).collect();

    assert_eq!(ids.len(), checks.len(), "{ids:?}");
}

#[test]
fn other_tools_and_commands_without_a_merge_are_silent() {
    for stdin in [
        r#"{"tool_name": "Edit", "tool_input": {"file_path": "x"}}"#,
        r#"{"tool_name": "Bash", "cwd": "/", "tool_input": {"command": "gh pr view 7"}}"#,
    ] {
        assert_eq!(gate(stdin.as_bytes()), "", "{stdin}");
    }
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn judging_a_head_with_no_verdict_or_one_git_cannot_find_denies_as_a_merge_would()
-> crate::test_repo::Setup<()> {
    use crate::pr_diff::Budget;
    use crate::test_repo::ScratchRepo;

    let repo = ScratchRepo::new()?;
    let head = repo.git(&["rev-parse", "HEAD"])?;
    let unknown = "0".repeat(40);
    let budget = Budget::per_call(crate::paths::GIT_TIMEOUT);

    let judge = |head: &str| {
        let pinned = super::Pinned { pr: 7, head };
        super::judge_head(repo.path(), |_: &str| None, pinned, budget)
    };

    let no_verdict = judge(&head);
    let not_local = judge(&unknown);

    let checks = [&no_verdict, &not_local].map(|denial| denial.as_ref().map(|d| d.check));
    assert_eq!(
        checks,
        [Some(Check::NoVerdict), Some(Check::Unjudged)],
        "{no_verdict:?} {not_local:?}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git to find the ledger")]
fn a_bash_input_without_a_command_is_denied() {
    let out = gate(br#"{"tool_name": "Bash", "cwd": "/nonexistent-anneal-test"}"#);

    assert!(out.contains("\"deny\""), "{out}");
    assert!(
        out.contains("ERROR merge-gate: the hook input cannot be read"),
        "{out}"
    );
}
