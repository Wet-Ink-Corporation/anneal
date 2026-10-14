//! The tables: how each wrapper the denylist's walk reads takes its words, as far as the
//! skeleton trusts them.

use super::after::After;
use super::shape::Shape;

/// How one wrapper reads the words between its name and what it runs, as far as the skeleton
/// trusts them. Each list is option words separated by spaces.
#[derive(Debug, Clone, Copy)]
pub(super) struct Reading {
    /// Options that take no value, written exactly (`-i`, `--login`).
    pub(super) flags: &'static str,
    /// Options that take the next word as their value, written exactly (`-u`, `--user`), or
    /// with the value attached (`-uroot`, `--user=root`) where [`Reading::style`] allows.
    pub(super) values: &'static str,
    /// The syntax of the value options of [`Reading::values`] that have one; any other value
    /// is [`Shape::Any`].
    pub(super) shapes: &'static [(&'static str, Shape)],
    /// Options whose value is the shell text the wrapper runs (`bash -c`, `flock -c`).
    pub(super) body: &'static str,
    /// Where a body option of [`Reading::body`] takes its body.
    pub(super) body_word: BodyWord,
    /// What may follow the body for the wrapper to run it.
    pub(super) after: After,
    /// Words after the options that are not the command (`timeout`'s duration).
    pub(super) operands: usize,
    /// The syntax of those operands, and of each operand standing among the options where
    /// [`Reading::operands_anywhere`] allows (`alias`'s definitions).
    pub(super) operand: Shape,
    /// Flags after which the operands are a [`Shape::CpuList`] instead (`taskset -c`).
    pub(super) lists: &'static str,
    /// Whether an operand may stand among the options (`su root -c`, `script file -c`).
    pub(super) operands_anywhere: bool,
    /// Whether `NAME=value` words after the options are the wrapper's own (`sudo K=v cmd`).
    pub(super) assignments: bool,
    /// How option words other than the exact ones read.
    pub(super) style: Style,
}

/// How a wrapper reads an option word that is none of its exact options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Style {
    /// As getopt does where the skeleton trusts it: a value option with its value attached
    /// (`-n1`, `--user=root`); any other word, a cluster included, is uncertain.
    Getopt,
    /// As a shell does: a cluster of its one-letter flags read letter by letter, a body
    /// option's letter only last (`bash -lc`), and no value attached, since a shell takes
    /// `-o`'s value from the next word whatever follows the `o` (`bash -oerrexit` reads the
    /// next word as the option's name).
    Shell,
}

/// Where a body option takes its body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BodyWord {
    /// Only the next word: flock compares its second operand with `-c` or `--command` whole,
    /// and runs `-cx` or `--command=x` as a program's name.
    Next,
    /// The next word, or the rest of the option's own (`-cbody`, `--command=body`), as getopt
    /// reads it.
    NextOrAttached,
}

/// A wrapper with no options, operands or assignments the skeleton trusts.
pub(super) const NONE: Reading = Reading {
    flags: "",
    values: "",
    shapes: &[],
    body: "",
    body_word: BodyWord::Next,
    after: After::Operands,
    operands: 0,
    operand: Shape::Any,
    lists: "",
    operands_anywhere: false,
    assignments: false,
    style: Style::Getopt,
};

/// bash: its one-letter flags, read letter by letter in a cluster, `-o` with the name of one of
/// its options as the next word, its long options (which bash takes only before the short
/// ones, [`Style::Shell`]), and `-c`, whose body is the next word once the options end. A
/// flag or `-o` name it does not know makes it refuse the command, `-n` reads the body
/// without running it, and `-O`'s names are not listed, so none of those is here.
const BASH: Reading = Reading {
    flags: "-a -b -B -C -e -E -f -h -H -i -k -l -m -p -P -r -t -T -u -v -x --login \
            --noediting --noprofile --norc --posix --restricted --verbose",
    values: "-o --rcfile --init-file",
    shapes: &[("-o", Shape::Words(BASH_OPTIONS))],
    body: "-c",
    style: Style::Shell,
    ..NONE
};

/// The other shells (`sh` among them, which may be any of them): only the flags and `-o`
/// names POSIX gives every shell, without `-n` (no run) and `-b` (zsh's end of options), since
/// dash refuses bash's other flags and long options, and mksh's `-T` takes the next word.
const POSIX_SHELL: Reading = Reading {
    flags: "-a -C -e -f -i -m -u -v -x",
    values: "-o",
    shapes: &[("-o", Shape::Words(POSIX_OPTIONS))],
    ..BASH
};

/// The `-o` names bash 5 takes, `noexec` left out (it runs nothing).
const BASH_OPTIONS: &str = "allexport braceexpand emacs errexit errtrace functrace hashall \
                            histexpand history ignoreeof interactive-comments keyword monitor \
                            noclobber noglob nolog notify nounset onecmd physical pipefail \
                            posix privileged verbose vi xtrace";

/// The `-o` names POSIX gives every shell, `noexec` and `nolog` left out.
const POSIX_OPTIONS: &str =
    "allexport errexit ignoreeof monitor noclobber noglob notify nounset verbose xtrace";

/// Each wrapper the denylist's walk reads, by name, and how the skeleton trusts its words.
/// `source`, `.` and `find` are absent: nothing after the first two is a program or a body,
/// and `find`'s words are read by [`super::find`]. Options that run nothing or act on
/// something else (`sudo -e`, `-l`, `-h`; `command -v`; `chrt` and `taskset -p`; `ionice -p`)
/// are left out, as are values a wrapper may take only attached (xargs's `-e`, `-i`, `-l`:
/// alone they are flags).
pub(super) const READINGS: [(&str, Reading); 34] = [
    (
        "sudo",
        Reading {
            flags: "-A -b -B -E -H -i -k -n -N -P -S -s --askpass --background --bell \
                    --preserve-env --set-home --login --reset-timestamp --non-interactive \
                    --no-update --preserve-groups --stdin --shell",
            values: "-C -D -g -p -r -t -T -u --close-from --chdir --group --prompt --role \
                     --type --command-timeout --user",
            shapes: &[
                ("-C", Shape::Digits),
                ("--close-from", Shape::Digits),
                ("-T", Shape::Duration),
                ("--command-timeout", Shape::Duration),
            ],
            assignments: true,
            ..NONE
        },
    ),
    (
        "doas",
        Reading {
            flags: "-n -s",
            values: "-u",
            ..NONE
        },
    ),
    (
        "env",
        Reading {
            flags: "-i -0 -v --ignore-environment --null --debug",
            values: "-u -C --unset --chdir",
            assignments: true,
            ..NONE
        },
    ),
    (
        "command",
        Reading {
            flags: "-p",
            ..NONE
        },
    ),
    ("nohup", NONE),
    ("builtin", NONE),
    (
        "setsid",
        Reading {
            flags: "-c -f -w --ctty --fork --wait",
            ..NONE
        },
    ),
    ("unbuffer", NONE),
    (
        "exec",
        Reading {
            flags: "-c -l",
            values: "-a",
            ..NONE
        },
    ),
    (
        "time",
        Reading {
            flags: "-p",
            ..NONE
        },
    ),
    (
        "nice",
        Reading {
            values: "-n --adjustment",
            shapes: &[("-n", Shape::Integer), ("--adjustment", Shape::Integer)],
            ..NONE
        },
    ),
    (
        "stdbuf",
        Reading {
            values: "-i -o -e --input --output --error",
            shapes: &[
                ("-i", Shape::Size),
                ("-o", Shape::Size),
                ("-e", Shape::Size),
                ("--input", Shape::Size),
                ("--output", Shape::Size),
                ("--error", Shape::Size),
            ],
            ..NONE
        },
    ),
    (
        "ionice",
        Reading {
            flags: "-t --ignore",
            values: "-c -n --class --classdata",
            shapes: &[
                ("-c", IO_CLASS),
                ("--class", IO_CLASS),
                ("-n", Shape::Digits),
                ("--classdata", Shape::Digits),
            ],
            ..NONE
        },
    ),
    (
        "timeout",
        Reading {
            flags: "-v --verbose --foreground --preserve-status",
            values: "-s -k --signal --kill-after",
            shapes: &[
                ("-s", Shape::Signal),
                ("--signal", Shape::Signal),
                ("-k", Shape::Duration),
                ("--kill-after", Shape::Duration),
            ],
            operands: 1,
            operand: Shape::Duration,
            ..NONE
        },
    ),
    (
        "chrt",
        Reading {
            flags: "-b -d -f -i -o -r -R -v --batch --deadline --fifo --idle --other --rr \
                    --reset-on-fork --verbose",
            values: "-T -P -D --sched-runtime --sched-period --sched-deadline",
            shapes: &[
                ("-T", Shape::Digits),
                ("-P", Shape::Digits),
                ("-D", Shape::Digits),
                ("--sched-runtime", Shape::Digits),
                ("--sched-period", Shape::Digits),
                ("--sched-deadline", Shape::Digits),
            ],
            // util-linux 2.42 takes the priority only when it is all digits, and needs none
            // under `-o`, `-b`, `-i` and `-d`.
            operands: 1,
            operand: Shape::Digits,
            ..NONE
        },
    ),
    (
        "taskset",
        Reading {
            flags: "-c --cpu-list",
            operands: 1,
            operand: Shape::Mask,
            lists: "-c --cpu-list",
            ..NONE
        },
    ),
    (
        "function",
        Reading {
            operands: 1,
            ..NONE
        },
    ),
    ("busybox", NONE),
    ("coproc", NONE),
    (
        "flock",
        Reading {
            flags: "-s -x -e -n -u -o -F --shared --exclusive --nonblock --unlock --close \
                    --no-fork --verbose",
            values: "-w -E --timeout --conflict-exit-code",
            shapes: &[
                ("-w", Shape::Number),
                ("--timeout", Shape::Number),
                ("-E", Shape::Digits),
                ("--conflict-exit-code", Shape::Digits),
            ],
            body: "-c --command",
            after: After::Nothing,
            operands: 1,
            ..NONE
        },
    ),
    (
        "xargs",
        Reading {
            flags: "-0 -e -i -l -o -p -r -t -x --null --no-run-if-empty --open-tty \
                    --interactive --verbose --exit",
            values: "-a -d -E -I -L -n -P -s --arg-file --delimiter --max-args --max-procs \
                     --max-chars --process-slot-var",
            shapes: &[
                ("-L", Shape::Digits),
                ("-n", Shape::Digits),
                ("-P", Shape::Digits),
                ("-s", Shape::Digits),
                ("--max-args", Shape::Digits),
                ("--max-procs", Shape::Digits),
                ("--max-chars", Shape::Digits),
            ],
            ..NONE
        },
    ),
    ("eval", NONE),
    (
        "alias",
        Reading {
            operand: Shape::Alias,
            operands_anywhere: true,
            ..NONE
        },
    ),
    (
        "trap",
        Reading {
            after: After::Signals,
            ..NONE
        },
    ),
    (
        "su",
        Reading {
            // `-s` (`--shell`) is left out: su runs the body with that program, which may be
            // no shell at all (`su -s /bin/false -c x`).
            flags: "- -f -l -m -p -P --login --preserve-environment --pty --fast",
            values: "-g -G -w --group --supp-group --whitelist-environment",
            body: "-c --command",
            body_word: BodyWord::NextOrAttached,
            operands_anywhere: true,
            ..NONE
        },
    ),
    (
        "script",
        Reading {
            // `-B`, `-I` and `-O` (`--log-io`, `--log-in`, `--log-out`) are left out: with one
            // of them script refuses a file operand, so the operands it allows are not one.
            flags: "-a -e -f -q -t --append --return --flush --quiet",
            values: "-E -T -m -o --echo --log-timing --logging-format --output-limit",
            shapes: &[
                ("-E", ECHO),
                ("--echo", ECHO),
                ("-m", LOGGING_FORMAT),
                ("--logging-format", LOGGING_FORMAT),
                ("-o", Shape::Size),
                ("--output-limit", Shape::Size),
            ],
            body: "-c --command",
            body_word: BodyWord::NextOrAttached,
            after: After::AtMostOneOperand,
            operands_anywhere: true,
            ..NONE
        },
    ),
    (
        "watch",
        Reading {
            flags: "-b -c -C -d -e -g -p -r -t -w --beep --color --no-color --differences \
                    --errexit --chgexit --precise --no-rerun --no-title --no-wrap",
            values: "-n -q --interval --equexit",
            shapes: &[
                ("-n", Shape::Number),
                ("--interval", Shape::Number),
                ("-q", Shape::Digits),
                ("--equexit", Shape::Digits),
            ],
            ..NONE
        },
    ),
    ("bash", BASH),
    ("sh", POSIX_SHELL),
    ("zsh", POSIX_SHELL),
    ("dash", POSIX_SHELL),
    ("ksh", POSIX_SHELL),
    ("mksh", POSIX_SHELL),
    ("ash", POSIX_SHELL),
];

/// `ionice -c`'s classes, by number or name.
const IO_CLASS: Shape = Shape::Words("0 1 2 3 none realtime best-effort idle");

/// `script -E`'s modes.
const ECHO: Shape = Shape::Words("auto always never");

/// `script -m`'s formats.
const LOGGING_FORMAT: Shape = Shape::Words("advanced classic");

/// The [`Reading`] of the wrapper `name`; `None` for one the tables do not know.
pub(super) fn reading(name: &str) -> Option<Reading> {
    READINGS
        .iter()
        .find(|(wrapper, _)| *wrapper == name)
        .map(|&(_, reading)| reading)
}
