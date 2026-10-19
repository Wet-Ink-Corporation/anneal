//! The real `gh` adapter's edges: how a failure is classified, the output cap, and the timeout.

use crate::dag::tracker::github::gh::{GhError, MAX_GH_OUTPUT, failure, within_cap};

#[test]
fn gh_saying_http_410_gone_is_not_found_too() {
    let gone = failure("repos/o/r/issues/9", b"gh: Gone (HTTP 410)\n");

    assert!(
        matches!(&gone, GhError::NotFound { endpoint } if endpoint == "repos/o/r/issues/9"),
        "a deleted issue is missing, like a 404: {gone:?}"
    );
}

#[test]
fn output_one_byte_under_the_cap_is_kept_and_at_the_cap_is_too_large() {
    let just_under = usize::try_from(MAX_GH_OUTPUT - 1).expect("fits");
    let at_cap = usize::try_from(MAX_GH_OUTPUT).expect("fits");

    let kept = within_cap("e", just_under);
    let refused = within_cap("e", at_cap);

    assert!(matches!(kept, Ok(())), "under the cap: {kept:?}");
    assert!(
        matches!(&refused, Err(GhError::TooLarge { endpoint }) if endpoint == "e"),
        "at the cap the answer may be cut short: {refused:?}"
    );
}

#[test]
fn the_gh_timeout_names_a_duration_under_a_second_in_whole_milliseconds() {
    use std::time::Duration;

    let short = GhError::Timeout {
        endpoint: "repos/o/r/issues".to_owned(),
        timeout: Duration::from_millis(500),
    };
    let left_over = GhError::Timeout {
        endpoint: "graphql".to_owned(),
        timeout: Duration::from_nanos(1_987_654_321),
    };

    assert_eq!(
        short.to_string(),
        "gh api repos/o/r/issues did not answer within 500ms"
    );
    assert_eq!(
        left_over.to_string(),
        "gh api graphql did not answer within 1.987s"
    );
}

#[test]
#[cfg(unix)]
#[cfg_attr(miri, ignore = "spawns a process")]
fn a_gh_that_never_answers_is_killed_at_the_timeout() -> Result<(), Box<dyn std::error::Error>> {
    use std::fs::{self, Permissions};
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;
    use std::time::Duration;

    use crate::dag::tracker::github::gh::{GhApi, GhCli};

    let scratch = tempfile::tempdir()?;
    // A gh that blocks opening a FIFO nobody ever writes, until it is killed.
    let fifo = scratch.path().join("never-written");
    let made = Command::new("mkfifo").arg(&fifo).status()?;
    assert!(made.success(), "mkfifo: {made:?}");
    let program = scratch.path().join("gh");
    fs::write(
        &program,
        format!("#!/bin/sh\nread line < '{}'\n", fifo.display()),
    )?;
    fs::set_permissions(&program, Permissions::from_mode(0o755))?;
    let gh = GhCli {
        program: program.into_os_string(),
        timeout: Duration::from_millis(50),
    };

    let result = gh.get("repos/o/r/issues");

    assert!(
        matches!(&result, Err(GhError::Timeout { endpoint, timeout })
            if endpoint == "repos/o/r/issues" && *timeout == Duration::from_millis(50)),
        "got {result:?}"
    );
    Ok(())
}

#[test]
fn a_graphql_query_sends_raw_fields_with_f_and_typed_ones_with_capital_f() {
    use crate::dag::tracker::github::gh::{Field, graphql_args};

    let args = graphql_args(
        "query { viewer { login } }",
        &[Field::Raw("ref", "@main"), Field::Typed("number", "5")],
    );

    assert_eq!(
        args,
        [
            "api",
            "graphql",
            "-f",
            "query=query { viewer { login } }",
            "-f",
            "ref=@main",
            "-F",
            "number=5",
        ]
        .map(std::ffi::OsString::from),
        "a raw field is never read as a file, number or placeholder"
    );
}
