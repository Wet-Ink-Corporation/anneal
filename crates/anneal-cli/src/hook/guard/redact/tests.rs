use super::{MAX_COMMAND_CHARS, REDACTED, redact};

#[test]
fn an_env_prefix_value_is_redacted() {
    assert_eq!(redact("FOO=bar cargo test"), "FOO=<redacted> cargo test");
}

#[test]
fn every_prefix_of_every_command_is_redacted_whatever_its_quoting() {
    let cases = [
        (
            "A=1 B='two words' C=\"x $(cat secret) y\" run",
            "A=<redacted> B=<redacted> C=<redacted> run",
        ),
        (
            "TOKEN=$(pass show ci) && X=y deploy",
            "TOKEN=<redacted> && X=<redacted> deploy",
        ),
        (
            "ls; K=v cmd | P=q other",
            "ls; K=<redacted> cmd | P=<redacted> other",
        ),
        ("echo hi\nK=v cmd", "echo hi\nK=<redacted> cmd"),
        ("(K=v cmd)", "(K=<redacted> cmd)"),
        ("K=a\\ b cmd", "K=<redacted> cmd"),
        ("K=$'a b' cmd", "K=<redacted> cmd"),
        ("K=`cat f` cmd", "K=<redacted> cmd"),
        ("K=${HOME}/x cmd", "K=<redacted> cmd"),
        (
            "if K=v cmd; then L=w x; fi",
            "if K=<redacted> cmd; then L=<redacted> x; fi",
        ),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn export_env_and_the_declaration_builtins_have_their_values_redacted() {
    let cases = [
        ("export API_KEY=abc", "export API_KEY=<redacted>"),
        ("export A=1 B=2", "export A=<redacted> B=<redacted>"),
        ("env -i A=b cmd x=y", "env -i A=<redacted> cmd x=y"),
        ("env A=b B=c cmd", "env A=<redacted> B=<redacted> cmd"),
        ("declare -x A=b", "declare -x A=<redacted>"),
        ("local A=b", "local A=<redacted>"),
        ("readonly A=b", "readonly A=<redacted>"),
        ("typeset A=b", "typeset A=<redacted>"),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn arguments_shaped_like_assignments_are_kept() {
    let cases = [
        "dd if=/dev/zero of=/dev/sda bs=1M",
        "make CC=gcc",
        "git commit -m \"A=b\"",
        "echo A=b",
        "git config user.name=x",
    ];
    for command in cases {
        assert_eq!(redact(command), command, "{command:?}");
    }
}

#[test]
fn values_of_credential_flags_are_redacted() {
    let cases = [
        (
            "curl --token=abc https://x",
            "curl --token=<redacted> https://x",
        ),
        (
            "login --password hunter2 --user me",
            "login --password <redacted> --user me",
        ),
        ("tool --api-key='k k' go", "tool --api-key=<redacted> go"),
        ("tool --API_KEY=k", "tool --API_KEY=<redacted>"),
        (
            "vault --client-secret s",
            "vault --client-secret <redacted>",
        ),
        ("gh --auth-token=t", "gh --auth-token=<redacted>"),
        ("tool --passwd p", "tool --passwd <redacted>"),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn flags_that_only_resemble_credentials_are_kept() {
    let cases = [
        "git commit --author='A <a@b>' -m x",
        "sort --key=2",
        "tool --password",
        "tool --password; ls",
    ];
    for command in cases {
        assert_eq!(redact(command), command, "{command:?}");
    }
}

#[test]
fn the_command_is_trimmed() {
    assert_eq!(redact("  \n ls -la \t\n"), "ls -la");
}

#[test]
fn a_long_command_is_cut_at_a_char_boundary_and_marked() {
    let command = format!("echo {}", "é".repeat(MAX_COMMAND_CHARS));

    let redacted = redact(&command);

    assert_eq!(
        redacted.chars().count(),
        MAX_COMMAND_CHARS + 1,
        "cut, plus the ellipsis"
    );
    assert!(redacted.starts_with("echo é"), "{redacted}");
    assert!(redacted.ends_with('…'), "marked as cut");
}

#[test]
fn values_are_redacted_before_the_cut_so_a_long_secret_does_not_crowd_out_the_command() {
    let command = format!("SECRET={} rm -r build", "h".repeat(2 * MAX_COMMAND_CHARS));

    assert_eq!(redact(&command), "SECRET=<redacted> rm -r build");
}

#[test]
fn an_unclosed_quote_redacts_to_the_end() {
    assert_eq!(redact("K='abc def"), format!("K={REDACTED}"));
}
