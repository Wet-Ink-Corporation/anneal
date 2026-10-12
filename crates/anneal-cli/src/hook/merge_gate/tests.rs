use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;

use super::{Check, pull, run};
use crate::hook::HookEnv;
use crate::hook::guard::{PrMerge, Target};

fn merge(target: Target, repo: Option<&str>, match_head: Option<&str>) -> PrMerge {
    PrMerge {
        target,
        repo: repo.map(str::to_owned),
        match_head: match_head.map(str::to_owned),
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

    assert_eq!(pull(&pinned), Ok((7, Some("abc"))));
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
#[cfg_attr(miri, ignore = "runs git to find the ledger")]
fn a_bash_input_without_a_command_is_denied() {
    let out = gate(br#"{"tool_name": "Bash", "cwd": "/nonexistent-anneal-test"}"#);

    assert!(out.contains("\"deny\""), "{out}");
    assert!(
        out.contains("ERROR merge-gate: the hook input cannot be read"),
        "{out}"
    );
}
