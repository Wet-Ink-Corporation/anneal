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
    /// Whether the wrapper surely runs what the walk places as its program or body.
    pub(super) runs: Runs,
}

/// Whether a wrapper surely runs the program or body the walk places after its words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Runs {
    /// It does, once its words read with certainty.
    Surely,
    /// Maybe not, or maybe something else: from that word on the command is arguments.
    /// `trap` sets no action when it refuses a signal (which signals it takes differs between
    /// shells and systems), `alias` runs its value only when a later command uses the name,
    /// `builtin` runs only a shell builtin and `busybox` only an applet of its own, and the
    /// word after `function NAME` or `coproc` may be no program (`function f x` is a syntax
    /// error; in `coproc NAME ( x )` the word is the coprocess's name).
    Maybe,
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
    runs: Runs::Surely,
};

/// bash: its one-letter flags, read letter by letter in a cluster, `-o` with the name of one of
/// its options as the next word, its long options (which bash takes only before the short
/// ones, [`Style::Shell`]), and `-c`, whose body is the next word once the options end. A
/// flag or `-o` name it does not know makes it refuse the command, `-n` reads the body
/// without running it, and `-O`'s names are not listed, so none of those is here; nor are
/// `-r` (`--restricted`), under which bash refuses a command name holding `/`, and `-k`
/// (`-o keyword`), under which it takes every assignment-shaped word out of the arguments
/// (`sudo -u X=1 psql` runs `sudo -u psql`).
const BASH: Reading = Reading {
    flags: "-a -b -B -C -e -E -f -h -H -i -l -m -p -P -t -T -u -v -x --login \
            --noediting --noprofile --norc --posix --verbose",
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

/// The `-o` names bash 5 takes, `noexec` (it runs nothing) and `keyword` (`-k`) left out.
const BASH_OPTIONS: &str = "allexport braceexpand emacs errexit errtrace functrace hashall \
                            histexpand history ignoreeof interactive-comments monitor \
                            noclobber noglob nolog notify nounset onecmd physical pipefail \
                            posix privileged verbose vi xtrace";

/// The `-o` names POSIX gives every shell, `noexec` and `nolog` left out.
const POSIX_OPTIONS: &str =
    "allexport errexit ignoreeof monitor noclobber noglob notify nounset verbose xtrace";

/// Each wrapper the denylist's walk reads, by name, and how the skeleton trusts its words.
/// `source`, `.` and `find` are absent: nothing after the first two is a program or a body,
/// and `find`'s words are read by [`super::find`]. So is `chrt`, whose priority's range
/// depends on its policy (`chrt -o 5` is refused). Options that run nothing or act on
/// something else (`sudo -e`, `-l`, `-h`; `command -v`; `taskset -p`; `ionice -p`), options
/// that make the wrapper refuse a command (`env -0`, `doas -s`) or a value it may refuse
/// whatever its syntax (`sudo -C 2`, `flock -E 300`, `xargs -d ab`), and options under which it
/// may not run the command at all (`xargs -p` asks first, `setsid -c` needs a terminal) are
/// left out, as are values a wrapper may take only attached (xargs's `-e`, `-l`: alone they
/// are flags).
pub(super) const READINGS: [(&str, Reading); 33] = [
    (
        "sudo",
        Reading {
            flags: "-A -b -B -E -H -i -k -n -N -P -S -s --askpass --background --bell \
                    --preserve-env --set-home --login --reset-timestamp --non-interactive \
                    --no-update --preserve-groups --stdin --shell",
            values: "-D -g -p -r -t -T -u --chdir --group --prompt --role --type \
                     --command-timeout --user",
            shapes: &[
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
            flags: "-n",
            values: "-u",
            ..NONE
        },
    ),
    (
        "env",
        Reading {
            flags: "-i -v --ignore-environment --debug",
            values: "-u -C --unset --chdir",
            shapes: &[("-u", Shape::Variable), ("--unset", Shape::Variable)],
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
    (
        "builtin",
        Reading {
            runs: Runs::Maybe,
            ..NONE
        },
    ),
    (
        "setsid",
        Reading {
            flags: "-f -w --fork --wait",
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
                ("-o", Shape::Buffering),
                ("-e", Shape::Buffering),
                ("--input", Shape::Size),
                ("--output", Shape::Buffering),
                ("--error", Shape::Buffering),
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
                ("-s", SIGNALS),
                ("--signal", SIGNALS),
                ("-k", Shape::Duration),
                ("--kill-after", Shape::Duration),
            ],
            operands: 1,
            operand: Shape::Duration,
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
            runs: Runs::Maybe,
            ..NONE
        },
    ),
    (
        "busybox",
        Reading {
            runs: Runs::Maybe,
            ..NONE
        },
    ),
    (
        "coproc",
        Reading {
            runs: Runs::Maybe,
            ..NONE
        },
    ),
    (
        "flock",
        Reading {
            flags: "-s -x -e -n -u -o -F --shared --exclusive --nonblock --unlock --close \
                    --no-fork --verbose",
            values: "-w --timeout",
            shapes: &[("-w", Shape::Number), ("--timeout", Shape::Number)],
            body: "-c --command",
            after: After::Nothing,
            operands: 1,
            ..NONE
        },
    ),
    (
        "xargs",
        Reading {
            // `-I`, `-i` and `--replace` are left out: xargs puts each input item in place of
            // the replace string in every argument, the program and a body included, so it
            // may run an input word (`xargs -I Z sh -c 'Z'`). So are `-s` (`--max-chars`),
            // refused when one argument does not fit, `-d`, `--process-slot-var` and `-p`.
            flags: "-0 -e -l -o -r -t -x --null --no-run-if-empty --open-tty --verbose --exit",
            values: "-a -E -L -n -P --arg-file --max-args --max-procs",
            shapes: &[
                ("-L", Shape::Count),
                ("-n", Shape::Count),
                ("-P", Shape::Digits),
                ("--max-args", Shape::Count),
                ("--max-procs", Shape::Digits),
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
            runs: Runs::Maybe,
            ..NONE
        },
    ),
    (
        "trap",
        Reading {
            runs: Runs::Maybe,
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

/// The signals `timeout -s` takes on every system: the POSIX names, bare or with `SIG`, and
/// their fixed numbers. Others (`RTMIN+1`, `65`) depend on the system, and `timeout` refuses
/// a signal it does not know and runs nothing.
const SIGNALS: Shape = Shape::Words(
    "HUP INT QUIT ABRT KILL ALRM TERM USR1 USR2 CONT STOP TSTP SIGHUP SIGINT SIGQUIT SIGABRT \
     SIGKILL SIGALRM SIGTERM SIGUSR1 SIGUSR2 SIGCONT SIGSTOP SIGTSTP 0 1 2 3 6 9 14 15",
);

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
