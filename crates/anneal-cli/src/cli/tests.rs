use super::{Cli, run};
use crate::command::CommandName;
use clap::CommandFactory;

/// Runs `anneal <args>` in memory: (exit code, stdout, stderr).
fn anneal(args: &[&str]) -> (u8, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = run(
        std::iter::once("anneal").chain(args.iter().copied()),
        &mut out,
        &mut err,
    );
    (
        code.value(),
        String::from_utf8(out).expect("stdout is UTF-8"),
        String::from_utf8(err).expect("stderr is UTF-8"),
    )
}

#[test]
fn the_clap_tree_is_consistent() {
    Cli::command().debug_assert();
}

#[test]
fn every_command_name_is_a_subcommand() {
    let tree = Cli::command();
    let names: Vec<&str> = tree
        .get_subcommands()
        .map(clap::Command::get_name)
        .collect();
    let expected: Vec<&str> = CommandName::ALL.iter().map(|c| c.as_str()).collect();
    assert_eq!(
        names, expected,
        "the tree matches CommandName::ALL, in order"
    );
}

#[test]
fn no_arguments_prints_help_and_exits_0() {
    let (code, out, err) = anneal(&[]);
    assert_eq!(code, 0, "bare `anneal` is a help request");
    assert!(out.starts_with("anneal:"), "help names the tool: {out}");
    assert!(err.is_empty(), "nothing on stderr");
}

#[test]
fn both_version_flags_print_the_package_version() {
    for flag in ["-V", "--version"] {
        let (code, out, err) = anneal(&[flag]);
        assert_eq!(code, 0, "{flag} succeeds");
        assert_eq!(
            out,
            format!("anneal {}\n", env!("CARGO_PKG_VERSION")),
            "{flag} prints the manifest version"
        );
        assert!(err.is_empty(), "{flag}: nothing on stderr");
    }
}

#[test]
fn both_help_flags_print_help_and_exit_0() {
    for flag in ["-h", "--help"] {
        let (code, out, err) = anneal(&[flag]);
        assert_eq!(code, 0, "{flag} succeeds");
        assert!(
            out.starts_with("anneal:"),
            "{flag} help names the tool: {out}"
        );
        assert!(err.is_empty(), "{flag}: nothing on stderr");
    }
}

#[test]
fn an_argument_after_any_help_or_version_flag_is_a_usage_error() {
    for flag in ["-h", "--help", "-V", "--version"] {
        let (code, out, err) = anneal(&[flag, "extra"]);
        assert_eq!(code, 2, "{flag} extra is a usage error");
        assert!(out.is_empty(), "{flag} extra: nothing on stdout");
        assert_eq!(
            err, "ERROR unexpected argument after --help or --version\n",
            "{flag} extra: exactly one ERROR line"
        );
    }
}

#[test]
fn top_level_help_lists_every_command_with_its_exit_codes() {
    let (code, out, _) = anneal(&["--help"]);
    assert_eq!(code, 0, "--help succeeds");
    for command in CommandName::ALL {
        let listed = out
            .lines()
            .filter(|l| l.trim_start().starts_with(command.as_str()))
            .count();
        assert!(
            listed >= 2,
            "{command} appears in the command list and the exit-code list"
        );
    }
    assert!(
        out.contains("64  not implemented yet"),
        "global codes shown"
    );
}

#[test]
fn command_help_shows_its_exit_codes_and_does_not_prove() {
    for command in CommandName::ALL {
        let (code, out, _) = anneal(&[command.as_str(), "--help"]);
        assert_eq!(code, 0, "{command} --help succeeds");
        assert!(out.contains("Exit codes ("), "{command} help shows codes");
        assert!(
            out.contains(command.claim().does_not_prove),
            "{command} help says what it does not prove"
        );
    }
}

#[test]
fn plan_help_says_only_the_plans_bytes_are_compared() {
    for args in [&["plan", "--help"][..], &["plan", "check", "--help"]] {
        let (code, out, _) = anneal(args);
        assert_eq!(code, 0, "{args:?} succeeds");
        assert!(
            out.contains("the plan's bytes"),
            "{args:?} names what is compared: {out}"
        );
        assert!(
            !out.contains("inputs"),
            "{args:?} claims no other inputs are compared: {out}"
        );
    }
}

#[test]
fn unknown_argument_exits_2_with_one_error_line() {
    let (code, out, err) = anneal(&["nope"]);
    assert_eq!(code, 2, "usage errors exit 2");
    assert!(out.is_empty(), "nothing on stdout for a usage error");
    assert_eq!(err, "ERROR unknown argument: nope\n", "exactly one line");
}

#[test]
fn unknown_flag_and_trailing_arguments_never_exit_0() {
    for args in [
        &["--frobnicate"][..],
        &["--version", "extra"],
        &["--help", "extra"],
    ] {
        let (code, out, err) = anneal(args);
        assert_ne!(code, 0, "{args:?} must not exit 0 (out: {out})");
        assert_eq!(err.lines().count(), 1, "{args:?}: one line: {err}");
        assert!(err.starts_with("ERROR "), "{args:?}: {err}");
    }
}

/// A command found under the tree: its path and the arguments it requires, as clap names them
/// in a usage error.
type Requiring = (Vec<String>, Vec<String>);

/// Every leaf command under `command` (at `path`) that requires an argument.
fn requiring_leaves(command: &clap::Command, path: &[String], found: &mut Vec<Requiring>) {
    let mut subcommands = command.get_subcommands().peekable();
    if subcommands.peek().is_none() {
        let required: Vec<String> = command
            .get_arguments()
            .filter(|arg| arg.is_required_set() && arg.get_default_values().is_empty())
            .map(ToString::to_string)
            .collect();
        if !required.is_empty() {
            found.push((path.to_vec(), required));
        }
        return;
    }
    for sub in subcommands {
        let mut deeper = path.to_vec();
        deeper.push(sub.get_name().to_owned());
        requiring_leaves(sub, &deeper, found);
    }
}

#[test]
fn a_missing_required_argument_is_named_for_every_command() {
    // Built, so each argument renders as clap renders it in the error.
    let mut tree = Cli::command();
    tree.build();
    let mut found = Vec::new();
    requiring_leaves(&tree, &[], &mut found);
    assert!(
        found.len() >= 20,
        "the walk reaches the commands that require an argument: {found:?}"
    );
    for expected in [["lint", "terms"], ["verdict", "set"], ["watch", "pr"]] {
        assert!(
            found
                .iter()
                .any(|(path, _)| path.iter().eq(expected.iter())),
            "the walk reaches {expected:?}: {found:?}"
        );
    }
    for (path, required) in found {
        let args: Vec<&str> = path.iter().map(String::as_str).collect();
        let (code, out, err) = anneal(&args);
        assert_eq!(code, 2, "{path:?}: usage error");
        assert!(out.is_empty(), "{path:?}: nothing on stdout");
        assert_eq!(err.lines().count(), 1, "{path:?}: one line: {err}");
        assert!(
            err.starts_with("ERROR missing required argument: "),
            "{path:?}: {err}"
        );
        for name in &required {
            assert!(err.contains(name.as_str()), "{path:?} names {name}: {err}");
        }
    }
}

#[test]
fn every_missing_required_argument_is_listed_comma_separated() {
    let command = clap::Command::new("t")
        .arg(clap::Arg::new("first").required(true))
        .arg(clap::Arg::new("second").long("second").required(true));
    let error = command
        .try_get_matches_from(["t"])
        .expect_err("both arguments are missing");

    assert_eq!(
        super::usage_reason(&error),
        "missing required argument: --second <second>, <first>",
        "every one, in clap's order (options before positionals)"
    );
}
