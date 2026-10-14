//! Review round 6 of PR 120: a word filling a wrapper's value or operand slot counts as
//! certain only when it has that slot's strict shape (chrt's priority all digits, timeout's
//! duration, taskset's mask, watch's interval, ...), a value word starting with `-` is no
//! certain value, a shell's `-o` takes no attached value, a shell flag cluster ends in `c`,
//! and `find`'s actions count only when every word of `find` before them reads with certainty.

use super::super::redact;
use super::ambiguous::whole_words;
use super::assert_cases;

/// Each command, the words of it that must not reach the output, and its skeleton.
const ROWS: &[(&str, &[&str], &str)] = &[
    // util-linux 2.42 takes chrt's priority only when it is all digits, and under `-o`,
    // `-b`, `-i` and `-d` none is needed: `mysql` is the command.
    (
        "chrt -i mysql -phunter2",
        &["mysql", "hunter2"],
        "chrt -i <redacted> -<redacted>",
    ),
    (
        "chrt -o mysql -phunter2",
        &["mysql", "hunter2"],
        "chrt -o <redacted> -<redacted>",
    ),
    (
        "chrt -b mysql -phunter2",
        &["mysql", "hunter2"],
        "chrt -b <redacted> -<redacted>",
    ),
    (
        "chrt -d mysql -phunter2",
        &["mysql", "hunter2"],
        "chrt -d <redacted> -<redacted>",
    ),
    (
        "chrt -f x10 psql",
        &["x10", "psql"],
        "chrt -f <redacted> <redacted>",
    ),
    (
        "chrt -T 5x -f 1 psql",
        &["psql"],
        "chrt -T <redacted> -f <redacted> <redacted>",
    ),
    // bash takes the next word for `-o` whatever follows it in the same word.
    (
        "bash -oerrexit -c 'sekritA argB'",
        &["sekritA", "argB"],
        "bash -<redacted> -c <redacted>",
    ),
    (
        "bash -Oextglob -c 'sekritA argB'",
        &["sekritA", "argB"],
        "bash -<redacted> -c <redacted>",
    ),
    // A shell's flag cluster is certain only as no-value flags with `c` last.
    (
        "bash -cl 'sekritA argB'",
        &["sekritA", "argB"],
        "bash -<redacted> <redacted>",
    ),
    (
        "bash -lsc 'sekritA argB'",
        &["sekritA", "argB"],
        "bash -<redacted> <redacted>",
    ),
    (
        "sh -eo x -c 'sekritA argB'",
        &["sekritA", "argB"],
        "sh -<redacted> <redacted> -c <redacted>",
    ),
    // A value that fails its option's syntax, or starts with `-`.
    (
        "watch -n cmdB argC",
        &["cmdB", "argC"],
        "watch -n <redacted> <redacted>",
    ),
    (
        "watch -n -d psql x",
        &["psql"],
        "watch -n -d <redacted> <redacted>",
    ),
    (
        "watch --interval=x psql",
        &["psql"],
        "watch --interval=<redacted> <redacted>",
    ),
    (
        "nice -n x5 psql",
        &["x5", "psql"],
        "nice -n <redacted> <redacted>",
    ),
    ("nice -nx5 psql", &["psql"], "nice -<redacted> <redacted>"),
    (
        "timeout 5x psql",
        &["psql"],
        "timeout <redacted> <redacted>",
    ),
    (
        "timeout -k x 5 psql",
        &["psql"],
        "timeout -k <redacted> <redacted> <redacted>",
    ),
    (
        "timeout -s 'x y' 5 psql",
        &["psql"],
        "timeout -s <redacted> <redacted> <redacted>",
    ),
    (
        "taskset 0-3 psql",
        &["psql"],
        "taskset <redacted> <redacted>",
    ),
    (
        "taskset 0xfg psql",
        &["psql"],
        "taskset <redacted> <redacted>",
    ),
    (
        "taskset -c ff psql",
        &["psql"],
        "taskset -c <redacted> <redacted>",
    ),
    (
        "ionice -c x psql",
        &["psql"],
        "ionice -c <redacted> <redacted>",
    ),
    (
        "ionice -n x psql",
        &["psql"],
        "ionice -n <redacted> <redacted>",
    ),
    (
        "xargs -n x psql",
        &["psql"],
        "xargs -n <redacted> <redacted>",
    ),
    ("xargs -P -t psql", &["psql"], "xargs -P -t <redacted>"),
    (
        "stdbuf -o x psql",
        &["psql"],
        "stdbuf -o <redacted> <redacted>",
    ),
    (
        "flock -w x f psql",
        &["psql"],
        "flock -w <redacted> <redacted> <redacted>",
    ),
    ("sudo -u -E psql", &["psql"], "sudo -u -E <redacted>"),
    ("sudo -C x psql", &["psql"], "sudo -C <redacted> <redacted>"),
    // find reads `-exec` as `-name`'s value, and `sekritA` as no action.
    (
        "find . -name -exec sekritA \\;",
        &["sekritA"],
        "find . -name -<redacted> <redacted> <redacted>",
    ),
    // `+` ends an action only after `{}`: here `-exec sekritA x` are echo's arguments.
    (
        "find . -exec echo + -exec sekritA x \\;",
        &["sekritA"],
        "find . -exec <redacted> <redacted> -<redacted> <redacted> <redacted> <redacted>",
    ),
    // A find word the tables do not know before an action.
    (
        "find . -fprintf f -exec sekritA x \\;",
        &["sekritA"],
        "find . -<redacted> <redacted> -<redacted> <redacted> <redacted> <redacted>",
    ),
    (
        "find . -maxdepth x -exec sekritA x \\;",
        &["sekritA"],
        "find . -maxdepth <redacted> -<redacted> <redacted> <redacted> <redacted>",
    ),
    // Review round 7: env splits `-S`'s string into words and runs them without a shell, so
    // `;` and backticks in it are arguments, not shell text.
    (
        "env -S 'echo a; hunter2'",
        &["hunter2"],
        "env -S <redacted>",
    ),
    (
        "env --split-string='echo a; hunter2'",
        &["hunter2"],
        "env --split-string=<redacted>",
    ),
    ("env -S 'echo `hunter2`'", &["hunter2"], "env -S <redacted>"),
    (
        "env -i -S 'echo a; hunter2'",
        &["hunter2"],
        "env -i -S <redacted>",
    ),
    // watch with `-x` runs its words without a shell.
    (
        "watch -x echo 'a; hunter2'",
        &["hunter2"],
        "watch -x <redacted> <redacted>",
    ),
    (
        "watch --exec echo 'a; hunter2'",
        &["hunter2"],
        "watch --exec <redacted> <redacted>",
    ),
    (
        "watch -n 1 -x echo 'a; hunter2'",
        &["hunter2"],
        "watch -n <redacted> -x <redacted> <redacted>",
    ),
    // A glob, variable or substitution in a value or operand slot may become several words
    // or none.
    (
        "sudo -u $U psql",
        &["psql"],
        "sudo -u <redacted> <redacted>",
    ),
    (
        "sudo -u \"$(whoami)\" psql",
        &["psql"],
        "whoami; sudo -u <redacted> <redacted>",
    ),
    ("nice -n $N rm", &["rm"], "nice -n <redacted> <redacted>"),
    (
        "doas -u u* psql",
        &["psql"],
        "doas -u <redacted> <redacted>",
    ),
    (
        "flock /tmp/*.lock psql",
        &["psql"],
        "flock <redacted> <redacted>",
    ),
    (
        "su $U -c 'dropdb x'",
        &["dropdb"],
        "su <redacted> -c <redacted>",
    ),
    (
        "find . -name *.rs -exec sekritA x \\;",
        &["sekritA"],
        "find . -name <redacted> -<redacted> <redacted> <redacted> <redacted>",
    ),
];

#[test]
fn a_value_or_operand_without_its_slot_s_shape_hides_the_rest_of_its_command() {
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

#[test]
fn a_value_or_operand_of_its_slot_s_shape_still_shows_what_it_runs() {
    assert_cases(&[
        ("chrt -o 0 psql x", "chrt -o <redacted> psql <redacted>"),
        ("chrt -i 0 mysql", "chrt -i <redacted> mysql"),
        (
            "chrt -T 1000 -f 1 psql",
            "chrt -T <redacted> -f <redacted> psql",
        ),
        ("timeout 1.5m psql x", "timeout <redacted> psql <redacted>"),
        (
            "timeout -k 5s -s TERM 10 psql",
            "timeout -k <redacted> -s <redacted> <redacted> psql",
        ),
        (
            "timeout --signal=SIGRTMIN+1 .5 psql",
            "timeout --signal=<redacted> <redacted> psql",
        ),
        ("taskset 0xff psql", "taskset <redacted> psql"),
        ("taskset 3 psql", "taskset <redacted> psql"),
        ("taskset -c 0-3,5,8-15:2 psql", "taskset -c <redacted> psql"),
        (
            "ionice -c idle -n 7 psql",
            "ionice -c <redacted> -n <redacted> psql",
        ),
        ("ionice -c2 psql", "ionice -<redacted> psql"),
        ("nice -n -5 psql", "nice -n -5 psql"),
        (
            "nice --adjustment=+5 psql",
            "nice --adjustment=<redacted> psql",
        ),
        (
            "watch -n 0.5 psql x",
            "watch -n <redacted> 'psql <redacted>'",
        ),
        (
            "stdbuf -o L -e 0 -i 4KiB psql",
            "stdbuf -o <redacted> -e <redacted> -i <redacted> psql",
        ),
        ("xargs -n 2 -P 4 rm", "xargs -n <redacted> -P <redacted> rm"),
        (
            "flock -w 1.5 -E 3 f psql",
            "flock -w <redacted> -E <redacted> <redacted> psql",
        ),
        (
            "sudo -C 3 -u root psql",
            "sudo -C <redacted> -u <redacted> psql",
        ),
        (
            "find . -mtime -7 -exec rm {} \\;",
            "find . -<redacted> -7 -exec rm {} ;",
        ),
        (
            "find -L / -maxdepth 2 -type f ! -name x -exec rm {} + -exec rm {} \\;",
            "find -L / -maxdepth <redacted> -type <redacted> <redacted> -name <redacted> -exec rm {} + \
             -exec rm {} ;",
        ),
    ]);
}

#[test]
fn a_shell_flag_cluster_ending_in_c_shows_its_body() {
    assert_cases(&[
        ("bash -lc 'rm -rf /'", "bash -<redacted> 'rm -rf /'"),
        ("sh -ec 'dropdb x'", "sh -<redacted> 'dropdb <redacted>'"),
        ("bash -xe -c 'rm -rf /'", "bash -<redacted> -c 'rm -rf /'"),
        (
            "bash -o errexit -c 'rm -rf /'",
            "bash -o <redacted> -c 'rm -rf /'",
        ),
    ]);
}
