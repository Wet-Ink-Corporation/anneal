use super::read;
use crate::hook::guard::{PrMerge, Target};

const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";

#[test]
fn the_canonical_merge_command_is_read_as_its_merge() {
    let command = format!("gh pr merge 7 --match-head-commit {HEAD} --repo github.com/o/r");

    assert_eq!(
        read(&command),
        Some(PrMerge {
            target: Target::Number(7),
            repo: Some("github.com/o/r".to_owned()),
            match_head: Some(HEAD.to_owned()),
            url_repo: None,
            env: None,
            moved: None,
        })
    );
}

#[test]
fn allowed_flags_texts_and_option_spellings_in_any_order_are_read() {
    let rows = [
        format!("gh pr merge --squash --repo github.com/o/r 7 --match-head-commit {HEAD}"),
        format!(
            "gh pr merge 7 --merge --delete-branch --auto --match-head-commit={HEAD} \
             -R github.com/o/r"
        ),
        format!("gh pr merge 7 --rebase --match-head-commit {HEAD} -Rgithub.com/o/r"),
        format!(
            "gh pr merge 7 --subject 'a $(b) \"c\"' --body \"plain text\" \
             --match-head-commit {HEAD} --repo=github.com/o/r"
        ),
        format!("\tgh  pr merge 7 --match-head-commit {HEAD} --repo github.com/o/r\n"),
        format!(
            "gh pr merge 7 --match-head-commit {HEAD} --repo github.com/x/y --repo github.com/o/r"
        ),
    ];

    for command in &rows {
        let merge = read(command);
        assert_eq!(
            merge.as_ref().map(|merge| (
                &merge.target,
                merge.repo.as_deref(),
                merge.match_head.as_deref()
            )),
            Some((&Target::Number(7), Some("github.com/o/r"), Some(HEAD))),
            "{command}"
        );
    }
}

#[test]
fn a_pull_request_url_or_a_missing_value_is_read_for_the_gate_to_judge() {
    let url = read("gh pr merge https://github.com/o/r/pull/7 --repo github.com/o/r");
    let bare = read("gh pr merge");

    assert_eq!(
        url.map(|merge| (merge.target, merge.url_repo, merge.match_head)),
        Some((Target::Number(7), Some("github.com/o/r".to_owned()), None))
    );
    assert_eq!(
        bare.map(|merge| (merge.target, merge.repo)),
        Some((Target::Missing, None))
    );
}

/// Commands holding a merge that the gate must not judge as written, each from a review of #89.
fn other_shapes() -> Vec<String> {
    let merge = format!("gh pr merge 7 --match-head-commit {HEAD} --repo github.com/o/r");
    let rest = format!("7 --match-head-commit {HEAD} --repo github.com/o/r");
    vec![
        // Separators, pipes, redirections, comments, assignments, wrappers, subshells.
        format!("{merge}; echo done"),
        format!("{merge} && echo done"),
        format!("echo | {merge}"),
        format!("{merge} > out"),
        format!("{merge} 2>&1"),
        format!("{merge} # merged"),
        format!("{merge}\necho done"),
        format!("GH_REPO=o/x {merge}"),
        format!("command {merge}"),
        format!("({merge})"),
        format!("\\{merge}"),
        format!("'gh' pr merge {rest}"),
        // Round 3: brace and pattern words, quoted lists.
        format!("{merge} --body {{a..b}}"),
        format!("{merge} --body {{a,b}}"),
        format!("gh pr merge 7 {{1..2}} --match-head-commit {HEAD} --repo github.com/o/r"),
        format!("{merge} --body-file ~/notes"),
        format!("{merge} --body *"),
        format!(r#"{merge} --body "$@""#),
        format!("{merge} --body $x"),
        format!(r#"{merge} --body "$(cat notes)""#),
        format!(r#"{merge} --body "`cat notes`""#),
        // Round 3: functions and aliases.
        format!(r#"f(){{ gh pr merge "$@"; }}; f {rest}"#),
        format!(r#"gh(){{ command gh "$@" --repo github.com/x/y; }}; {merge}"#),
        format!("alias gh='gh --repo github.com/x/y'; {merge}"),
        // Round 4 F1: a quoted word joined to a brace or glob.
        format!(
            r#"gh pr merge 7 --body "$b"{{x,--repo=github.com/x/y}} --match-head-commit {HEAD} --repo github.com/o/r"#
        ),
        format!(r#"{merge} --body "$b"*"#),
        format!("{merge} --body 'a'{{x,y}}"),
        // Round 4 F2: arguments appended at run time.
        format!("cat args | xargs {merge}"),
        format!("xargs -a args {merge}"),
        format!("xargs {merge} < args"),
        format!("find . -name x -exec {merge} {{}} +"),
        // Round 4 F3: another gh, or definitions read before the merge.
        format!("hash -p ./evil/gh gh; {merge}"),
        format!("PATH=./evil:$PATH {merge}"),
        format!("export PATH=./evil:$PATH; {merge}"),
        format!(". ./defs; {merge}"),
        format!("source ./defs; {merge}"),
        format!("BASH_ENV=./defs bash -c '{merge}'"),
        format!("bash --rcfile ./defs -i -c '{merge}'"),
        format!("enable -f ./evil.so gh; {merge}"),
        // Words and options outside the allowlist.
        format!("gh pr merge 7 6 --match-head-commit {HEAD} --repo github.com/o/r"),
        format!("gh pr merge #7 --match-head-commit {HEAD} --repo github.com/o/r"),
        format!("{merge} --disable-auto"),
        format!("{merge} -d"),
        format!("{merge} --body-file notes"),
        format!("{merge} --admin"),
        format!("{merge} -- 8"),
        format!(r#"gh pr merge 7 --match-head-commit {HEAD} --repo "github.com/o/r""#),
        format!("gh pr merge 7 --match-head-commit {HEAD} --repo"),
        format!("{merge} --subject"),
        format!(r#"{merge} --subject "unclosed"#),
        format!("gh pr merge '7' --match-head-commit {HEAD} --repo github.com/o/r"),
        format!("gh -R github.com/o/r pr merge 7 --match-head-commit {HEAD}"),
    ]
}

/// Texts and endings the reader refuses: a double-quoted text holding `\` or `!`, a trailing
/// CR or NUL, quoted parts joined into one word, and one word past [`super::MAX_WORDS`].
fn refused_texts_and_endings() -> Vec<String> {
    let merge = format!("gh pr merge 7 --match-head-commit {HEAD} --repo github.com/o/r");
    vec![
        format!(r#"{merge} --body "a\"b""#),
        format!(r#"{merge} --body "a!b""#),
        format!("{merge}\r"),
        format!("{merge}\0"),
        format!("{merge} --body 'a''b'"),
        format!("{merge}{}", " --squash".repeat(25)),
    ]
}

#[test]
fn a_command_of_exactly_the_word_limit_is_read() {
    let merge = format!("gh pr merge 7 --match-head-commit {HEAD} --repo github.com/o/r");
    let command = format!("{merge}{}", " --squash".repeat(24));

    let read = read(&command);

    assert_eq!(
        (command.split(' ').count(), read.map(|merge| merge.target)),
        (super::MAX_WORDS, Some(Target::Number(7))),
        "{command}"
    );
}

#[test]
fn one_word_past_the_limit_is_not_read() {
    let merge = format!("gh pr merge 7 --match-head-commit {HEAD} --repo github.com/o/r");
    let command = format!("{merge}{}", " --squash".repeat(25));

    assert_eq!(
        (command.split(' ').count(), read(&command)),
        (super::MAX_WORDS + 1, None),
        "{command}"
    );
}

#[test]
fn any_other_command_holding_a_merge_is_not_read() {
    for command in other_shapes()
        .into_iter()
        .chain(refused_texts_and_endings())
    {
        assert_eq!(read(&command), None, "{command}");
    }
}
