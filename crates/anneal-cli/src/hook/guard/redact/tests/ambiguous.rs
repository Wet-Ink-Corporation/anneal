//! Review round 5 of PR 120: the skeleton is built on the denylist's walk as main reads it,
//! and where that walk may place the program wrongly (a short-option cluster, an option the
//! tables do not know, chrt or taskset acting on a pid) the rest of the simple command is
//! rendered as arguments.

use super::super::redact;
use super::assert_cases;

/// Each command, the words of it that must not reach the output, and its skeleton.
const ROWS: &[(&str, &[&str], &str)] = &[
    (
        "xargs -eE psql hunter2",
        &["psql", "hunter2"],
        "xargs -<redacted> <redacted> <redacted>",
    ),
    (
        "watch -dn psql hunter2",
        &["psql", "hunter2"],
        "watch -<redacted> <redacted> <redacted>",
    ),
    (
        "script -tqc x pw52",
        &["x", "pw52"],
        "script -<redacted> <redacted> <redacted>",
    ),
    (
        "sudo -iu zqUSER psql",
        &["zqUSER", "psql"],
        "sudo -<redacted> <redacted> <redacted>",
    ),
    (
        "echo 0 | xargs chrt -p 10 zqSECRET 4242",
        &["zqSECRET", "4242"],
        "echo <redacted> | xargs chrt -p <redacted> <redacted> <redacted>",
    ),
    // After a literal `--` every word is an operand, past the uncertain word too.
    (
        "su -- -c --zqSECRET",
        &["zqSECRET"],
        "su -- <redacted> <redacted>",
    ),
];

#[test]
fn a_wrapper_read_with_certainty_still_shows_what_it_runs() {
    assert_cases(&[
        ("sudo -u root -i psql", "sudo -u <redacted> -i psql"),
        ("sudo --user=root -E psql", "sudo --user=<redacted> -E psql"),
        ("xargs -0 -n 1 rm -rf", "xargs -0 -n <redacted> rm -rf"),
        (
            "xargs -n1 -I % rm -rf %",
            "xargs -<redacted> -I <redacted> rm -rf <redacted>",
        ),
        (
            "env -i -u HOME A=1 rm -rf /",
            "env -i -u <redacted> A=<redacted> rm -rf /",
        ),
        (
            "timeout --signal=KILL 5 rm -rf /",
            "timeout --signal=<redacted> <redacted> rm -rf /",
        ),
        (
            "nice -n 5 ionice -c 3 chrt -f 10 taskset -c 0 rm -rf /",
            "nice -n <redacted> ionice -c <redacted> chrt -f <redacted> taskset -c <redacted> \
             rm -rf /",
        ),
        ("su - root -c 'rm -rf /'", "su - <redacted> -c 'rm -rf /'"),
        (
            "flock -x /tmp/l -c 'rm -rf /'",
            "flock -x <redacted> -c 'rm -rf /'",
        ),
        ("bash -e -c 'rm -rf /'", "bash -e -c 'rm -rf /'"),
        (
            "script -q -c 'rm -rf /' /dev/null",
            "script -q -c 'rm -rf /' /dev/null",
        ),
        ("watch -n 1 rm -rf /", "watch -n <redacted> 'rm -rf /'"),
        (
            "env --split-string='rm -rf /'",
            "env --split-string=<redacted> 'rm -rf /'",
        ),
        ("sudo -- rm -rf /", "sudo -- rm -rf /"),
    ]);
}

/// The whole words of `text`: its runs of ASCII letters, digits and `_`.
pub(super) fn whole_words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|word| !word.is_empty())
}

#[test]
fn a_word_after_an_ambiguous_wrapper_option_is_never_shown_as_a_program() {
    let wrong: Vec<String> = ROWS
        .iter()
        .filter_map(|&(command, secrets, expected)| {
            let redacted = redact(command);
            let shown: Vec<&str> = secrets
                .iter()
                .copied()
                .filter(|secret| whole_words(&redacted).any(|word| word == *secret))
                .collect();
            (redacted != expected || !shown.is_empty())
                .then(|| format!("{command:?} became {redacted:?}, showing {shown:?}"))
        })
        .collect();

    assert!(wrong.is_empty(), "misrendered:\n{}", wrong.join("\n"));
}
