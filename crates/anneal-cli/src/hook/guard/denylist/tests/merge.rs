//! `find_pr_merge`: the merge gate's probe for `gh pr merge`, through the guard's checker.

use super::context;
use crate::hook::guard::Rule;
use crate::hook::guard::denylist::{MergeScan, PrMerge, Target, find_pr_merge};

fn merges(command: &str) -> Vec<PrMerge> {
    match find_pr_merge(command, &context()) {
        MergeScan::Read { merges, .. } => merges,
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

/// The one merge `command` runs: its target, `-R`/`--repo`, and the URL's repository.
fn read_one(command: &str) -> (Target, Option<String>, Option<String>) {
    let found = merges(command);
    let [merge] = found.as_slice() else {
        panic!("{command:?}: one merge expected, found {found:?}");
    };
    (
        merge.target.clone(),
        merge.repo.clone(),
        merge.url_repo.clone(),
    )
}

#[test]
fn a_short_option_cluster_takes_its_value_as_pflag_does() {
    let rows = [
        ("gh pr merge -db 5 6", Target::Number(6), None),
        ("gh pr merge -sdt subject 4", Target::Number(4), None),
        ("gh pr merge -dA me@example.com 8", Target::Number(8), None),
        ("gh pr merge -sbbody 9", Target::Number(9), None),
        ("gh pr merge -dRo/x 3", Target::Number(3), Some("o/x")),
        ("gh pr merge -R=o/x 3", Target::Number(3), Some("o/x")),
    ];

    for (command, target, repo) in rows {
        let (found, found_repo, _) = read_one(command);
        assert_eq!((found, found_repo.as_deref()), (target, repo), "{command}");
    }
}

#[test]
fn options_before_pr_are_read_with_the_same_table() {
    let rows = [
        ("gh -R o/x pr merge 7", Some("o/x")),
        ("gh --repo=o/x pr merge 7", Some("o/x")),
        ("gh --squash=true pr merge 7", None),
    ];

    for (command, repo) in rows {
        let (target, found_repo, _) = read_one(command);
        assert_eq!(
            (target, found_repo.as_deref()),
            (Target::Number(7), repo),
            "{command}"
        );
    }
}

#[test]
fn only_a_true_disable_auto_turns_the_merge_off() {
    for command in [
        "gh pr merge 3 --disable-auto=false",
        "gh pr merge 3 --disable-auto=0",
        "gh pr merge 3 --disable-auto=no",
    ] {
        assert_eq!(targets(command), [Target::Number(3)], "{command}");
    }
    for command in [
        "gh pr merge 3 --disable-auto=true",
        "gh pr merge 3 --disable-auto=1",
        "gh pr merge 3 --disable-auto=T",
    ] {
        assert_eq!(targets(command), [], "{command}");
    }
}

#[test]
fn only_a_pull_request_url_names_a_number_and_its_repository_is_kept() {
    let rows = [
        (
            "gh pr merge https://GitHub.com/O/R/pull/42",
            Target::Number(42),
            Some("github.com/o/r"),
        ),
        (
            "gh pr merge https://github.com/o/r/pull/42#discussion",
            Target::Number(42),
            Some("github.com/o/r"),
        ),
        ("gh pr merge feature/pull/5", Target::Unnumbered, None),
        (
            "gh pr merge https://github.com/o/pull/5",
            Target::Unnumbered,
            None,
        ),
        (
            "gh pr merge https://github.com/o/r/pull/5x",
            Target::Unnumbered,
            None,
        ),
        (
            "gh pr merge ftp://github.com/o/r/pull/5",
            Target::Unnumbered,
            None,
        ),
        ("gh pr merge 42", Target::Number(42), None),
    ];

    for (command, target, url_repo) in rows {
        let (found, _, found_url) = read_one(command);
        assert_eq!(
            (found, found_url.as_deref()),
            (target, url_repo),
            "{command}"
        );
    }
}

#[test]
fn a_gh_repository_or_host_set_in_the_command_is_noted() {
    let rows = [
        ("GH_REPO=o/x gh pr merge 7", "GH_REPO"),
        ("env GH_HOST=ghe.example gh pr merge 7", "GH_HOST"),
        ("export GH_REPO=o/x; gh pr merge 7", "GH_REPO"),
        ("bash -c 'GH_REPO=o/x gh pr merge 7'", "GH_REPO"),
    ];

    for (command, name) in rows {
        let found = merges(command);
        let names: Vec<Option<&str>> = found.iter().map(|merge| merge.env.as_deref()).collect();
        assert_eq!(names, [Some(name)], "{command}");
    }
    let plain = merges("gh pr merge 7");
    assert_eq!(
        plain
            .iter()
            .map(|merge| merge.env.as_deref())
            .collect::<Vec<_>>(),
        [None],
        "nothing set"
    );
}

#[test]
fn a_gh_repository_or_host_set_through_brace_expansion_is_noted() {
    let rows = [
        ("export GH_RE{PO,X}=o/x; gh pr merge 5", "GH_REPO"),
        ("readonly GH_{HOST,X}=ghe.example; gh pr merge 5", "GH_HOST"),
        ("declare -x {GH_REPO,X}=o/x; gh pr merge 5", "GH_REPO"),
    ];

    for (command, name) in rows {
        let found = merges(command);
        let names: Vec<Option<&str>> = found.iter().map(|merge| merge.env.as_deref()).collect();
        assert_eq!(names, [Some(name)], "{command}");
    }
    // `env` takes a word with a brace for its program, which the checker cannot read.
    let scan = find_pr_merge("env GH_{HOST,X}=ghe.example gh pr merge 5", &context());
    assert!(matches!(scan, MergeScan::Unreadable(_)), "{scan:?}");
}

/// How each merge `command` runs was moved to another directory, in order.
fn moved(command: &str) -> Vec<Option<&'static str>> {
    merges(command).iter().map(|merge| merge.moved).collect()
}

#[test]
fn a_directory_change_before_gh_is_noted_on_the_merge() {
    let rows = [
        ("cd /tmp && gh pr merge 5", "cd"),
        ("pushd /tmp; gh pr merge 5", "pushd"),
        ("popd; gh pr merge 5", "popd"),
        ("bash -c 'cd ../other && gh pr merge 5'", "cd"),
        ("env -C /tmp gh pr merge 5", "env --chdir"),
        ("env --chdir=/tmp gh pr merge 5", "env --chdir"),
        ("sudo -D /tmp gh pr merge 5", "sudo --chdir"),
        ("nohup env -iC /tmp gh pr merge 5", "env --chdir"),
        ("env -C /tmp bash -c 'gh pr merge 5'", "env --chdir"),
        ("find . -execdir gh pr merge 5 ';'", "find -execdir"),
    ];

    for (command, how) in rows {
        assert_eq!(moved(command), [Some(how)], "{command}");
    }
}

#[test]
fn a_directory_change_anywhere_in_the_command_is_noted_on_the_merge() {
    // A function or trap body is read where it is defined, but runs later.
    let rows = [
        "gh pr merge 5 && cd /tmp",
        "f(){ gh pr merge 5; }; cd /other && f",
        "trap 'gh pr merge 5' EXIT; cd /other",
    ];

    for command in rows {
        assert_eq!(moved(command), [Some("cd")], "{command}");
    }
}

#[test]
fn a_login_shell_of_sudo_or_su_is_noted_as_a_directory_change() {
    let rows = [
        ("sudo -i gh pr merge 5", "sudo --login"),
        ("sudo --login gh pr merge 5", "sudo --login"),
        ("sudo -iu me gh pr merge 5", "sudo --login"),
        ("su - me -c 'gh pr merge 5'", "su --login"),
        ("su -l -c 'gh pr merge 5'", "su --login"),
        ("su --login me -c 'gh pr merge 5'", "su --login"),
    ];

    for (command, how) in rows {
        assert_eq!(moved(command), [Some(how)], "{command}");
    }
    assert_eq!(moved("su me -c 'gh pr merge 5'"), [None], "no login shell");
}

#[test]
fn a_gh_variable_built_by_a_brace_sequence_is_unreadable() {
    let rows = [
        "export GH_REP{O..O}=o/x; gh pr merge 5",
        "env GH_HO{S..S}T=ghe.example gh pr merge 5",
        "export {G..G}H_REPO=o/x; gh pr merge 5",
        "export G{H..H}_REPO=o/x; gh pr merge 5",
    ];

    for command in rows {
        let scan = find_pr_merge(command, &context());
        assert!(
            matches!(scan, MergeScan::Unreadable(ref denial) if denial.rule == Rule::Unparseable),
            "{command}: {scan:?}"
        );
    }
    assert_eq!(
        targets("echo {1..3}; gh pr merge 5"),
        [Target::Number(5)],
        "a sequence that cannot name a GH_ variable"
    );
}

#[test]
fn a_gh_variable_built_by_a_brace_sequence_beside_an_expansion_is_unreadable() {
    let rows = [
        "export GH_REP{O..O}\"$x\"=o/x; gh pr merge 5",
        "export G{H..H}_REPO$x=o/x; gh pr merge 5",
    ];

    for command in rows {
        let scan = find_pr_merge(command, &context());
        assert!(
            matches!(scan, MergeScan::Unreadable(ref denial) if denial.rule == Rule::Unparseable),
            "{command}: {scan:?}"
        );
    }
}

#[test]
fn a_merge_run_where_the_command_started_is_not_moved() {
    let rows = [
        "gh pr merge 5",
        "env GH_PROMPT_DISABLED=1 gh pr merge 5",
        "sudo -u me gh pr merge 5",
        "env -C /tmp ls; gh pr merge 5",
    ];

    for command in rows {
        assert_eq!(moved(command), [None], "{command}");
    }
}

#[test]
fn a_call_the_probe_cannot_read_is_unseen_when_the_command_may_merge() {
    let gh_call = "a gh call whose subcommand is known only at run time";
    let program = "a program known only at run time";
    let definition = "a function, alias or hash -p definition";
    let rows = [
        // A gh call whose first or second operand is known only at run time, merge or not.
        (r#"gh "$@""#, Some(gh_call)),
        ("gh $x view 7", Some(gh_call)),
        (r#"f() { :; }; gh "$sub""#, Some(gh_call)),
        // A run-time program or a definition, when the text holds merge in any case.
        ("$x pr merge 7", Some(program)),
        ("g$x pr MERGE 7", Some(program)),
        ("f() { :; }; echo merge", Some(definition)),
        ("alias g=gh; g pr merge 7", Some(definition)),
        (
            "env 'BASH_FUNC_g%%=() { :; }' bash -c 'g merge'",
            Some(definition),
        ),
        // Without merge in the text, or with every program and gh call written out.
        ("$EDITOR x", None),
        ("f() { :; }; f", None),
        ("alias ll='ls -l'", None),
        (r#"gh pr view "$x""#, None),
        ("echo merge; git merge main", None),
        // A parenthesis that opens no definition: an array, a subshell after a keyword, a
        // case pattern.
        ("a=(x y); echo merge", None),
        ("if (true); then echo merge; fi", None),
        ("case x in (x) echo merge;; esac", None),
    ];

    for (command, expected) in rows {
        assert_eq!(unseen(command), expected, "{command}");
    }
}

/// How `command`, which runs no merge the probe reads, may run one it cannot read.
fn unseen(command: &str) -> Option<&'static str> {
    match find_pr_merge(command, &context()) {
        MergeScan::Read { merges, unseen } if merges.is_empty() => unseen,
        other => panic!("{command}: {other:?}"),
    }
}

#[test]
fn a_word_naming_gh_beside_merge_trips_unless_the_probe_read_that_gh_call() {
    let gh_word = "gh where the probe cannot read its arguments";
    let rows = [
        // A gh the probe reads no call from: appended arguments, an unknown program's
        // arguments, a path, a quote that splits merge, a name looked up.
        ("xargs -a merge-args gh", Some(gh_word)),
        ("cp \"$(command -v gh)\" g; ./g pr merge 7", Some(gh_word)),
        ("parallel gh ::: pr ::: merge ::: 7", Some(gh_word)),
        ("ln -s /usr/bin/gh g; ./g pr merge 7", Some(gh_word)),
        ("echo gh pr merge 3", Some(gh_word)),
        ("echo 'mer'ge | xargs GH.EXE", Some(gh_word)),
        // A gh call taken from a variable does not cover another command's unread gh.
        (
            "G=gh; $G pr view 1; echo pr merge 7 > f; cat f | xargs gh",
            Some(gh_word),
        ),
        (
            r#"x="gh pr view 1"; $x; echo merge > f; cat f | xargs gh"#,
            Some(gh_word),
        ),
        // Every gh read as a call, or no word naming gh exactly.
        ("gh pr merge 3 --disable-auto", None),
        ("gh pr view 7 && git merge gh-pages", None),
        ("/usr/bin/gh pr checks 7; git merge main", None),
        ("echo gh", None),
    ];

    for (command, expected) in rows {
        assert_eq!(unseen(command), expected, "{command}");
    }
}

#[test]
fn a_gh_call_read_again_with_its_expansions_empty_is_counted_once() {
    // `"$b"*` gives the call a second reading with `$b` empty; counting the call there too
    // would pair it with the second word naming gh.
    let gh_word = "gh where the probe cannot read its arguments";
    let rows = [
        ("gh pr view 1 \"$b\"* gh merge", Some(gh_word)),
        ("gh pr view 1 \"$b\"* merge", None),
    ];

    for (command, expected) in rows {
        assert_eq!(unseen(command), expected, "{command}");
    }
}

#[test]
fn a_gh_api_call_trips_only_when_a_word_holds_merge() {
    let gh_api = "gh api, which may call the merge endpoint";
    let rows = [
        ("gh api -X PUT repos/o/r/pulls/7/merge", Some(gh_api)),
        ("gh -R o/r api -X PUT repos/o/r/pulls/7/MERGE", Some(gh_api)),
        ("gh api repos/o/r/pulls/7", None),
        ("gh api repos/o/r/pulls/7 && git status", None),
    ];

    for (command, expected) in rows {
        assert_eq!(unseen(command), expected, "{command}");
    }
}

#[test]
fn hash_p_is_a_definition_bare_or_under_builtin_or_command() {
    let definition = "a function, alias or hash -p definition";
    for command in [
        "hash -p /tmp/x g; g merge",
        "builtin hash -p /tmp/x g; g merge",
        "command hash -rp /tmp/x g; g merge",
    ] {
        assert_eq!(unseen(command), Some(definition), "{command}");
    }
    assert_eq!(
        unseen("hash -r; echo merge"),
        None,
        "hash -r defines nothing"
    );
}

#[test]
fn a_merge_the_probe_finds_is_read_even_beside_a_call_it_cannot_read() {
    let found = merges(r#"gh "$@"; gh pr merge 7"#);

    assert_eq!(
        found.iter().map(|merge| &merge.target).collect::<Vec<_>>(),
        [&Target::Number(7)]
    );
}

#[test]
fn an_env_short_cluster_before_a_merge_is_read_as_main_reads_it() {
    // PR 120's redaction review: the probe reads `-vu` as one flag and `X` as env's program,
    // as main (603fa75) does, so env's `-C` is not seen as a directory change (the merge gate
    // denies the command as `merge-gate.shape`, not `merge-gate.directory`).
    let command = "env -vu X -C /tmp gh pr merge 7";

    assert_eq!(moved(command), [None], "{command}");
}

#[test]
fn a_command_the_guard_cannot_read_is_unreadable_here_too() {
    let scan = find_pr_merge("gh pr merge 'unclosed", &context());

    assert!(
        matches!(scan, MergeScan::Unreadable(ref denial) if denial.rule == Rule::Unparseable),
        "{scan:?}"
    );
}
