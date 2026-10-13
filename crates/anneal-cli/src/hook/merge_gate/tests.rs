use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;

use super::pull::{Pull, check_repo, origin_repo, pull};
use super::{Check, Screen, run, screen};
use crate::hook::HookEnv;
use crate::hook::guard::{Context, PrMerge, Target};

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

/// How the gate screens `command` before reading git: `Some(check)` when it denies, `None`
/// when it judges the command's merge.
fn screened(command: &str) -> Option<Check> {
    match screen(command, &Context::new(|_: &str| None)) {
        Screen::Deny(denial) => Some(denial.check),
        Screen::Judge(_) => None,
        Screen::NoMerge => panic!("{command}: no merge found"),
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
            "git@github.com:Owner/Repo.GIT\n",
            Some("github.com/owner/repo"),
        ),
        ("https://github.com/o/r$x.git", None),
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
fn a_repository_named_as_host_owner_and_repository_passes_to_the_gate() {
    let named = merge(
        Target::Number(7),
        Some("github.com/owner/repo"),
        Some("abc"),
    );

    assert_eq!(
        pull(&named),
        Ok(Pull {
            pr: 7,
            pin: Some("abc"),
            url_repo: None
        })
    );
}

#[test]
fn a_merge_whose_repository_may_differ_at_run_time_is_denied() {
    let command = r#"r=github.com/owner/repo; gh pr merge 7 --match-head-commit abc --repo "$r""#;

    let Screen::Deny(denial) = screen(command, &Context::new(|_: &str| None)) else {
        panic!("{command}: not denied");
    };

    assert_eq!(
        (denial.check, denial.reason.contains(super::canonical::FORM)),
        (Check::Shape, true),
        "{denial:?}"
    );
}

#[test]
fn a_merge_whose_repo_or_pin_may_differ_at_run_time_is_denied_unless_written_out() {
    let rows = [
        // A variable's value the guard put in, which control flow or unset may change.
        (
            r#"r=github.com/owner/repo; gh pr merge 5 --repo "$r""#,
            Some(Check::Shape),
        ),
        (
            "r=github.com/owner/repo; gh pr merge 5 --repo $r",
            Some(Check::Shape),
        ),
        (
            r#"false && r=github.com/owner/repo; gh pr merge 5 --repo "$r""#,
            Some(Check::Shape),
        ),
        (
            r#"r=github.com/owner/repo; unset r; gh pr merge 5 --repo "$r""#,
            Some(Check::Shape),
        ),
        (
            "r=github.com/owner/repo; gh pr merge 5 --repo=$r",
            Some(Check::Shape),
        ),
        (
            r#"h=abc; gh pr merge 5 --match-head-commit "$h" -R github.com/owner/repo"#,
            Some(Check::Shape),
        ),
        // A word that may vanish or split, shifting which word --repo takes.
        (
            "gh pr merge 5 --body $x --repo github.com/owner/repo",
            Some(Check::Shape),
        ),
        (
            "gh pr merge 5 --body * --repo github.com/owner/repo",
            Some(Check::Shape),
        ),
        // More than one operand.
        (
            "gh pr merge 5 $x --repo github.com/owner/repo",
            Some(Check::Shape),
        ),
        (
            "gh pr merge 5 6 --repo github.com/owner/repo",
            Some(Check::Shape),
        ),
        // Written out: judged.
        (
            "gh pr merge 5 --match-head-commit abc --repo github.com/owner/repo",
            None,
        ),
        ("gh pr merge 5 -R github.com/owner/repo", None),
        // A quoted expansion stays one word but runs or reads something: denied too.
        (
            r#"gh pr merge 5 --body "$(cat notes)" --repo github.com/owner/repo"#,
            Some(Check::Shape),
        ),
        (
            r#"x=1; gh pr merge 5 --body "$x" --repo github.com/owner/repo"#,
            Some(Check::Shape),
        ),
    ];

    for (command, check) in rows {
        assert_eq!(screened(command), check, "{command}");
    }
}

#[test]
fn a_merge_with_a_word_that_is_not_literal_or_quoted_is_denied() {
    let rows = [
        // A brace sequence or list, which bash splits into several words.
        (
            "gh pr merge 5 --body {a..b} --repo github.com/owner/repo",
            Check::Shape,
        ),
        (
            "gh pr merge 5 --body {a,b} --repo github.com/owner/repo",
            Check::Shape,
        ),
        (
            "gh pr merge 5 {1..2} --repo github.com/owner/repo",
            Check::Shape,
        ),
        (
            "gh pr merge 5 {a,b} --repo github.com/owner/repo",
            Check::Shape,
        ),
        // Any other pattern word, `~` included.
        (
            "gh pr merge 5 --body-file ~/notes --repo github.com/owner/repo",
            Check::Shape,
        ),
        // A quoted expansion that may still split or vanish.
        (
            r#"bash -c 'gh pr merge 5 --repo github.com/owner/repo "$@"' _ --repo o/x"#,
            Check::Shape,
        ),
        (
            r#"a=(x); gh pr merge 5 --body "${a[@]}" --repo github.com/owner/repo"#,
            Check::Shape,
        ),
        // gh's first operand known only at run time: no pull request number either.
        (
            r#"gh "$g" merge 5 --repo github.com/owner/repo"#,
            Check::Target,
        ),
    ];

    for (command, check) in rows {
        assert_eq!(screened(command), Some(check), "{command}");
    }
}

#[test]
fn a_merge_in_a_command_defining_a_function_or_alias_is_denied() {
    let merge = "gh pr merge 5 --match-head-commit abc --repo github.com/owner/repo";
    // A merge read in a body names no pull request when the body takes it from the call.
    let rows = [
        (
            r#"f(){ gh pr merge "$@"; }; f 5 --repo github.com/other/repo"#.to_owned(),
            Check::Target,
        ),
        (
            format!(r#"gh(){{ command gh "$@" --repo github.com/other/repo; }}; {merge}"#),
            Check::Shape,
        ),
        (
            format!(r#"gh () {{ command gh "$@"; }}; {merge}"#),
            Check::Shape,
        ),
        (
            format!(r#"function gh {{ command gh "$@"; }}; {merge}"#),
            Check::Shape,
        ),
        (
            "alias f='gh pr merge'; f 5 --repo github.com/other/repo".to_owned(),
            Check::Target,
        ),
        (
            format!("alias gh='gh --repo github.com/other/repo'; {merge}"),
            Check::Shape,
        ),
        (
            format!("builtin alias gh='gh -R o/x'; {merge}"),
            Check::Shape,
        ),
        (
            format!(r"env 'BASH_FUNC_gh%%=() {{ :; }}' bash -c '{merge}'"),
            Check::Shape,
        ),
        (format!("{merge}; f() {{ :; }}"), Check::Shape),
    ];

    for (command, check) in &rows {
        assert_eq!(screened(command), Some(*check), "{command}");
    }
}

#[test]
fn a_merge_after_a_parenthesis_is_denied_as_not_its_own_command() {
    let merge = "gh pr merge 5 --match-head-commit abc --repo github.com/owner/repo";
    let rows = [
        (format!("a=(x y); {merge}"), Check::Shape),
        (format!("if (true); then {merge}; fi"), Check::Shape),
        (format!("case x in (x) {merge};; esac"), Check::Shape),
        (format!("(cd .); {merge}"), Check::Directory),
    ];

    for (command, check) in &rows {
        assert_eq!(screened(command), Some(*check), "{command}");
    }
}

#[test]
fn every_bypass_from_the_fourth_review_of_89_is_denied_before_git_is_read() {
    let pin = "--match-head-commit abc --repo github.com/owner/repo";
    let merge = format!("gh pr merge 7 {pin}");
    let rows = [
        // F1: a quoted word joined to a brace or glob.
        format!(r#"gh pr merge 7 --body "$b"{{x,--repo=github.com/other/repo}} {pin}"#),
        format!(r#"{merge} --body "$b"*"#),
        // F2: arguments appended at run time.
        format!("cat args | xargs {merge}"),
        format!("xargs -a args {merge}"),
        format!("xargs {merge} < args"),
        format!("find . -name x -exec {merge} {{}} +"),
        // F3: another gh, or definitions read before the merge.
        format!("hash -p ./evil/gh gh; {merge}"),
        format!("PATH=./evil:$PATH {merge}"),
        format!("export PATH=./evil:$PATH; {merge}"),
        format!(". ./defs; {merge}"),
        format!("source ./defs; {merge}"),
        format!("BASH_ENV=./defs bash -c '{merge}'"),
        format!("bash --rcfile ./defs -i -c '{merge}'"),
        format!("enable -f ./evil.so gh; {merge}"),
    ];

    for command in &rows {
        assert_eq!(screened(command), Some(Check::Shape), "{command}");
    }
}

/// Commands that run a merge the probe cannot see, from the fifth review of #89, and a lone
/// gh call whose subcommand is known only at run time.
const UNSEEN_MERGES: [&str; 14] = [
    r#"gh "$@""#,
    r#"f() { gh "$@"; }; f pr merge 7"#,
    "alias g=gh\ng pr merge 7",
    "alias g='gh pr'\ng merge 7",
    r#"set -- pr merge 7; gh "$@""#,
    r#"arr=(pr merge 7); gh "${arr[@]}""#,
    r#"read -r x <<< "pr merge 7"; gh $x"#,
    "gh $(echo pr merge) 7",
    "gh $(cat args)",
    "gh `cat args`",
    "${GH:-gh} pr merge 7",
    "${GH-gh} pr merge 7",
    "g$x pr merge 7",
    "f() { gh \"$@\"; }; f pr MERGE 7",
];

#[test]
fn a_merge_the_probe_cannot_see_is_denied_by_its_shape() {
    for command in UNSEEN_MERGES {
        assert_eq!(screened(command), Some(Check::Shape), "{command}");
    }
}

/// Commands whose words name `merge` and gh, or `merge` and a definition, around a merge the
/// probe cannot read (the sixth review of #89). The probe reads the alias body
/// `gh pr mer''ge` as a merge naming no pull request, so that one is denied as `target`. The
/// seventh review added gh words a run-time call hides and a `gh api` call to the merge endpoint.
const TRIPWIRE_MERGES: [(&str, Check); 17] = [
    (r"printf '%s\n' pr merge 7 | xargs gh", Check::Shape),
    (r"printf 'pr\nmerge\n7\n' | xargs gh", Check::Shape),
    ("echo pr merge 7 > f; xargs gh < f", Check::Shape),
    ("echo pr merge 7 > f; cat f | xargs gh", Check::Shape),
    ("echo merge | xargs -I% gh pr % 7", Check::Shape),
    ("alias g=gh\ng pr \"merg\"e 7", Check::Shape),
    ("alias g='gh pr'\ng 'mer'ge 7", Check::Shape),
    ("alias g='gh pr'\ng me\\rge 7", Check::Shape),
    ("alias g='gh pr mer''ge'\ng 7", Check::Target),
    ("hash -p /usr/bin/gh g; g pr merge 7", Check::Shape),
    ("builtin hash -p /usr/bin/gh g; g pr merge 7", Check::Shape),
    (
        r#"cp "$(command -v gh)" /tmp/g && /tmp/g pr merge 7"#,
        Check::Shape,
    ),
    ("ln -sf /usr/bin/gh /tmp/g; /tmp/g pr merge 7", Check::Shape),
    ("parallel gh ::: pr ::: merge ::: 7", Check::Shape),
    (
        "G=gh; $G pr view 1; echo pr merge 7 > f; cat f | xargs gh",
        Check::Shape,
    ),
    (
        r#"x="gh pr view 1"; $x; echo merge > f; cat f | xargs gh"#,
        Check::Shape,
    ),
    ("gh api -X PUT repos/o/r/pulls/7/merge", Check::Shape),
];

#[test]
fn a_command_naming_merge_and_gh_or_a_definition_is_denied() {
    for (command, check) in TRIPWIRE_MERGES {
        assert_eq!(screened(command), Some(check), "{command}");
    }
}

#[test]
fn a_git_merge_naming_no_gh_word_is_allowed() {
    for command in ["git merge gh-pages", "git merge --no-ff feature"] {
        let screened = screen(command, &Context::new(|_: &str| None));

        assert_eq!(screened, Screen::NoMerge, "{command}");
    }
}

#[test]
fn a_gh_api_call_naming_no_merge_is_allowed() {
    let screened = screen("gh api repos/o/r/pulls/7", &Context::new(|_: &str| None));

    assert_eq!(screened, Screen::NoMerge);
}

#[test]
fn a_run_time_program_or_a_definition_without_merge_in_the_command_is_allowed() {
    for command in ["$EDITOR x", "f(){ :; }; f", "alias ll='ls -l'"] {
        let screened = screen(command, &Context::new(|_: &str| None));

        assert_eq!(screened, Screen::NoMerge, "{command}");
    }
}

#[test]
fn the_canonical_merge_command_is_judged() {
    let command = "gh pr merge 7 --squash --match-head-commit abc --repo github.com/owner/repo";

    assert_eq!(screened(command), None, "{command}");
}

#[test]
fn a_repository_not_named_as_host_owner_and_repository_is_denied() {
    let rows = [
        "owner/repo",
        "https://github.com/owner/repo",
        "github.com/owner/repo.git",
        "github.com/owner/repo.GIT",
        "github.com/owner/repo/extra",
        "github.com//repo",
        "github.com/owner/$REPO",
        "github.com/owner/{repo,other}",
        "github.com/owner/rep*",
        "",
    ];

    for repo in rows {
        let denial = pull(&merge(Target::Number(7), Some(repo), None)).expect_err(repo);
        assert_eq!(
            (denial.check, denial.reason.contains("HOST/OWNER/REPO")),
            (Check::Repo, true),
            "{repo}: {denial:?}"
        );
    }
}

#[test]
fn a_merge_must_name_the_judged_repository_with_repo() {
    let origin = "github.com/owner/repo";
    let command = "`gh pr merge 7 --match-head-commit abc --repo github.com/owner/repo`";
    let rows = [
        (Some("github.com/owner/repo"), None),
        (Some("GitHub.com/Owner/Repo"), None),
        (None, Some("names no --repo")),
        (
            Some("github.com/other/repo"),
            Some("names github.com/other/repo"),
        ),
        (
            Some("ghe.example/owner/repo"),
            Some("names ghe.example/owner/repo"),
        ),
    ];

    for (repo, denied) in rows {
        let denial = check_repo(7, "abc", repo, Some(origin));
        let found = denial
            .as_ref()
            .map(|denial| (denial.check, denial.reason.as_str()));
        match denied {
            None => assert_eq!(found, None, "{repo:?}"),
            Some(why) => {
                let (check, reason) = found.expect("denied");
                assert_eq!(
                    (check, reason.contains(why), reason.contains(command)),
                    (Check::Repo, true, true),
                    "{repo:?}: {reason}"
                );
            }
        }
    }
}

#[test]
fn a_merge_in_a_repository_without_a_readable_origin_is_denied() {
    let denial = check_repo(7, "abc", Some("github.com/owner/repo"), None).expect("denied");

    assert_eq!(
        (denial.check, denial.reason.contains("origin")),
        (Check::Repo, true),
        "{denial:?}"
    );
}

#[test]
fn every_check_has_its_own_id() {
    let checks = [
        Check::BadInput,
        Check::Unreadable,
        Check::Shape,
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
