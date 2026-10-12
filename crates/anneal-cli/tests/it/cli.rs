//! The built binary answers `--help` and `--version` and rejects anything else with exit 2.

use std::error::Error;
use std::process::Command;

type TestResult = Result<(), Box<dyn Error>>;

fn anneal() -> Command {
    Command::new(env!("CARGO_BIN_EXE_anneal"))
}

#[test]
fn help_exits_0_and_names_the_tool() -> TestResult {
    let output = anneal().arg("--help").output()?;

    assert!(output.status.success(), "--help should exit 0: {output:?}");
    assert!(
        String::from_utf8(output.stdout)?.starts_with("anneal:"),
        "help names the tool"
    );
    Ok(())
}

#[test]
fn version_matches_the_manifest() -> TestResult {
    let output = anneal().arg("--version").output()?;

    assert!(
        output.status.success(),
        "--version should exit 0: {output:?}"
    );
    assert_eq!(
        String::from_utf8(output.stdout)?,
        format!("anneal {}\n", env!("CARGO_PKG_VERSION")),
        "--version prints the manifest version"
    );
    Ok(())
}

#[test]
fn unknown_argument_exits_2_and_writes_nothing_on_stdout() -> TestResult {
    let output = anneal().arg("frobnicate").output()?;

    assert_eq!(
        output.status.code(),
        Some(2),
        "usage errors exit 2: {output:?}"
    );
    assert!(
        output.stdout.is_empty(),
        "stdout stays empty on a usage error"
    );
    Ok(())
}

#[test]
fn a_usage_error_writes_exactly_one_error_line() -> TestResult {
    let output = anneal().arg("frobnicate").output()?;

    assert_eq!(
        String::from_utf8(output.stderr)?,
        "ERROR unknown argument: frobnicate\n",
        "the whole of stderr is one ERROR line, no usage dump"
    );
    Ok(())
}

#[test]
fn help_lists_every_command_with_its_exit_codes() -> TestResult {
    let output = anneal().arg("--help").output()?;
    let help = String::from_utf8(output.stdout)?;

    assert!(output.status.success(), "--help exits 0");
    for line in [
        "  floor-guard         0 no floor lowered · 1 a floor was lowered · 2 tool error",
        "  64  not implemented yet",
    ] {
        assert!(
            help.lines().any(|l| l == line),
            "missing {line:?} in:\n{help}"
        );
    }
    Ok(())
}

#[test]
fn arguments_after_version_are_a_usage_error() -> TestResult {
    let output = anneal().args(["--version", "extra"]).output()?;

    assert_eq!(output.status.code(), Some(2), "never 0: {output:?}");
    assert!(output.stdout.is_empty(), "nothing on stdout");
    Ok(())
}
