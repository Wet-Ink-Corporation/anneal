//! `find_pr_merge`: the merge gate's probe for `gh pr merge`, through the guard's checker.

use super::context;
use crate::hook::guard::Rule;
use crate::hook::guard::denylist::{MergeScan, PrMerge, Target, find_pr_merge};

fn merges(command: &str) -> Vec<PrMerge> {
    match find_pr_merge(command, &context()) {
        MergeScan::Merges(merges) => merges,
        MergeScan::Unreadable(denial) => panic!("{command:?} read as unreadable: {denial:?}"),
    }
}

fn targets(command: &str) -> Vec<Target> {
    merges(command)
        .into_iter()
        .map(|merge| merge.target)
        .collect()
}

#[test]
fn the_pull_request_is_read_from_a_number_a_hash_number_or_a_url() {
    let rows = [
        ("gh pr merge 42", Target::Number(42)),
        ("gh pr merge '#42' --squash", Target::Number(42)),
        (
            "gh pr merge https://github.com/o/r/pull/42/files --merge",
            Target::Number(42),
        ),
        ("gh pr merge --squash --delete-branch 7", Target::Number(7)),
        ("gh pr merge -b 'body text' 9", Target::Number(9)),
        ("gh pr merge -- 11", Target::Number(11)),
    ];

    for (command, expected) in rows {
        assert_eq!(targets(command), [expected], "{command}");
    }
}

#[test]
fn a_merge_that_names_no_pull_request_by_number_is_flagged() {
    let rows = [
        ("gh pr merge", Target::Missing),
        ("gh pr merge --squash", Target::Missing),
        ("gh pr merge feature-branch", Target::Unnumbered),
        ("gh pr merge \"$PR\"", Target::Unnumbered),
        ("gh pr $(echo merge) 3", Target::Unnumbered),
    ];

    for (command, expected) in rows {
        assert_eq!(targets(command), [expected], "{command}");
    }
}

#[test]
fn the_repo_flag_and_the_head_commit_are_read() {
    let found = merges("gh pr merge -R o/r --match-head-commit abc123 5");
    let attached = merges("gh pr merge --repo=o/r --match-head-commit=def 5");

    assert_eq!(
        (
            found
                .iter()
                .map(|merge| (merge.repo.as_deref(), merge.match_head.as_deref()))
                .collect::<Vec<_>>(),
            attached
                .iter()
                .map(|merge| (merge.repo.as_deref(), merge.match_head.as_deref()))
                .collect::<Vec<_>>(),
        ),
        (
            vec![(Some("o/r"), Some("abc123"))],
            vec![(Some("o/r"), Some("def"))]
        ),
        "both spellings"
    );
}

#[test]
fn every_wrapping_form_the_guard_reads_still_finds_the_merge() {
    let forms = [
        "env GH_PROMPT_DISABLED=1 gh pr merge 3",
        "sudo -u me gh pr merge 3",
        "bash -c 'gh pr merge 3'",
        "eval gh pr merge 3",
        "cd sub && /usr/local/bin/gh pr merge 3",
        "timeout 30 gh pr merge 3 --squash",
        "echo 3 | xargs gh pr merge",
        "caffeinate gh pr merge 3",
        "bash <<'EOF'\ngh pr merge 3\nEOF",
        "X=gh; $X pr merge 3",
    ];

    for command in forms {
        assert_eq!(targets(command), [Target::Number(3)], "{command}");
    }
}

#[test]
fn every_merge_in_a_command_is_found() {
    assert_eq!(
        targets("gh pr merge 1 && gh pr merge 2"),
        [Target::Number(1), Target::Number(2)],
        "the second merge cannot hide behind the first"
    );
}

#[test]
fn other_commands_and_disabling_auto_merge_are_not_merges() {
    let rows = [
        "gh pr view 3",
        "gh pr checks 3",
        "echo gh pr merge 3",
        "gh pr merge 3 --disable-auto",
        "git merge main",
        "rm -rf /",
    ];

    for command in rows {
        assert_eq!(targets(command), [], "{command}");
    }
}

#[test]
fn a_command_the_guard_cannot_read_is_unreadable_here_too() {
    let scan = find_pr_merge("gh pr merge 'unclosed", &context());

    assert!(
        matches!(scan, MergeScan::Unreadable(ref denial) if denial.rule == Rule::Unparseable),
        "{scan:?}"
    );
}
