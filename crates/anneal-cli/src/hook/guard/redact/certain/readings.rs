//! The tables: how each wrapper the denylist's walk reads takes its words, as far as the
//! skeleton trusts them.

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
    /// Options whose value is the shell text the wrapper runs (`bash -c`, `env -S`).
    pub(super) body: &'static str,
    /// Words after the options that are not the command (`timeout`'s duration).
    pub(super) operands: usize,
    /// The syntax of those operands.
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

/// A wrapper with no options, operands or assignments the skeleton trusts.
pub(super) const NONE: Reading = Reading {
    flags: "",
    values: "",
    shapes: &[],
    body: "",
    operands: 0,
    operand: Shape::Any,
    lists: "",
    operands_anywhere: false,
    assignments: false,
    style: Style::Getopt,
};

/// The shells: one-letter flags, read letter by letter in a cluster, `-o`/`-O` with the next
/// word as their value, and `-c`, whose body is the next word once the options end.
const SHELL: Reading = Reading {
    flags: "-a -b -B -C -e -E -f -h -H -i -k -l -m -n -p -P -r -t -T -u -v -x --login \
            --noediting --noprofile --norc --posix --restricted --verbose",
    values: "-o -O --rcfile --init-file",
    body: "-c",
    style: Style::Shell,
    ..NONE
};

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
            body: "-S --split-string",
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
            operands_anywhere: true,
            ..NONE
        },
    ),
    ("trap", NONE),
    (
        "su",
        Reading {
            flags: "- -f -l -m -p -P --login --preserve-environment --pty --fast",
            values: "-s -g -G -w --shell --group --supp-group --whitelist-environment",
            body: "-c --command",
            operands_anywhere: true,
            ..NONE
        },
    ),
    (
        "script",
        Reading {
            flags: "-a -e -f -q -t --append --return --flush --quiet",
            values: "-B -E -I -O -T -m -o --log-io --echo --log-in --log-out --log-timing \
                     --logging-format --output-limit",
            shapes: &[
                ("-E", ECHO),
                ("--echo", ECHO),
                ("-m", LOGGING_FORMAT),
                ("--logging-format", LOGGING_FORMAT),
                ("-o", Shape::Size),
                ("--output-limit", Shape::Size),
            ],
            body: "-c --command",
            operands_anywhere: true,
            ..NONE
        },
    ),
    (
        "watch",
        Reading {
            flags: "-b -c -C -d -e -g -p -r -t -w -x --beep --color --no-color \
                    --differences --errexit --chgexit --precise --no-rerun --no-title \
                    --no-wrap --exec",
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
    ("bash", SHELL),
    ("sh", SHELL),
    ("zsh", SHELL),
    ("dash", SHELL),
    ("ksh", SHELL),
    ("mksh", SHELL),
    ("ash", SHELL),
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
