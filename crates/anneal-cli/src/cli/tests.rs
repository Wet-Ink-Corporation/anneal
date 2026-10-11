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
fn a_stub_exits_64_with_one_error_line_and_no_stdout() {
    for args in [
        &["ledger"][..],
        &["ledger", "append", "--kind", "x"],
        &["lint", "spec", "-q"],
    ] {
        let (code, out, err) = anneal(args);
        assert_eq!(code, 64, "{args:?} is not implemented");
        assert!(out.is_empty(), "{args:?}: nothing on stdout");
        let command = args.first().expect("non-empty");
        assert_eq!(
            err,
            format!("ERROR anneal {command} is not implemented yet\n"),
            "{args:?}: one ERROR line"
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
