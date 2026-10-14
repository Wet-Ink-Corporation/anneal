//! Where the skeleton stops trusting the denylist's walk. The walk ([`denylist::layout`])
//! reads wrapper options loosely: an option it does not know is a flag, a short-option
//! cluster (`-iu`) is one flag, and chrt or taskset with `-p` runs the words after the
//! priority. That is the denylist's reading, kept as main has it; where it differs from the
//! wrapper's own, the word it places as the program may be an option's value (`sudo -iu
//! alice psql` reads `alice` as the program). So a wrapper's words are trusted only when they
//! read with certainty by the tables here: each an exact flag, an exact value option followed
//! by its value (or with it attached, `-n1`, `--user=x`), `--`, then the wrapper's fixed
//! operands and, for `sudo` and `env`, assignments, ending exactly where the walk put the next
//! wrapper, the program or the body. From the first word that does not, the rest of the
//! simple command is rendered as arguments: the skeleton may hide more than needed, never
//! show a value as a program.
//!
//! The tables trust only what they hold: a wrapper or option not listed is uncertain, and a
//! run of words is certain only where the tables and the walk read it the same way.

use super::denylist::{self, Role};
use super::shell::Word;

/// How one wrapper reads the words between its name and what it runs, as far as the skeleton
/// trusts them. Each list is option words separated by spaces.
#[derive(Debug, Clone, Copy)]
struct Reading {
    /// Options that take no value, written exactly (`-i`, `--login`).
    flags: &'static str,
    /// Options that take the next word as their value, written exactly (`-u`, `--user`), or
    /// with the value attached (`-uroot`, `--user=root`).
    values: &'static str,
    /// Options whose value is the shell text the wrapper runs (`bash -c`, `env -S`).
    body: &'static str,
    /// Words after the options that are not the command (`timeout`'s duration).
    operands: usize,
    /// Whether an operand may stand among the options (`su root -c`, `script file -c`).
    operands_anywhere: bool,
    /// Whether `NAME=value` words after the options are the wrapper's own (`sudo K=v cmd`).
    assignments: bool,
}

/// A wrapper with no options, operands or assignments the skeleton trusts.
const NONE: Reading = Reading {
    flags: "",
    values: "",
    body: "",
    operands: 0,
    operands_anywhere: false,
    assignments: false,
};

/// The shells: one-letter flags, `-o`/`-O` with a value, and `-c`, whose body is the next
/// word once the options end.
const SHELL: Reading = Reading {
    flags: "-a -b -B -C -e -E -f -h -H -i -k -l -m -n -p -P -r -t -T -u -v -x --login \
            --noediting --noprofile --norc --posix --restricted --verbose",
    values: "-o -O --rcfile --init-file",
    body: "-c",
    ..NONE
};

/// Each wrapper the denylist's walk reads, by name, and how the skeleton trusts its words.
/// `source`, `.` and `find` are absent: nothing after them is a program or a body. Options
/// that run nothing or act on something else (`sudo -e`, `-l`, `-h`; `command -v`; `chrt` and
/// `taskset -p`; `ionice -p`) are left out, as are values a wrapper may take only attached
/// (xargs's `-e`, `-i`, `-l`: alone they are flags).
const READINGS: [(&str, Reading); 34] = [
    (
        "sudo",
        Reading {
            flags: "-A -b -B -E -H -i -k -n -N -P -S -s --askpass --background --bell \
                    --preserve-env --set-home --login --reset-timestamp --non-interactive \
                    --no-update --preserve-groups --stdin --shell",
            values: "-C -D -g -p -r -t -T -u --close-from --chdir --group --prompt --role \
                     --type --command-timeout --user",
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
            ..NONE
        },
    ),
    (
        "stdbuf",
        Reading {
            values: "-i -o -e --input --output --error",
            ..NONE
        },
    ),
    (
        "ionice",
        Reading {
            flags: "-t --ignore",
            values: "-c -n --class --classdata",
            ..NONE
        },
    ),
    (
        "timeout",
        Reading {
            flags: "-v --verbose --foreground --preserve-status",
            values: "-s -k --signal --kill-after",
            operands: 1,
            ..NONE
        },
    ),
    (
        "chrt",
        Reading {
            flags: "-b -d -f -i -o -r -R -v --batch --deadline --fifo --idle --other --rr \
                    --reset-on-fork --verbose",
            values: "-T -P -D --sched-runtime --sched-period --sched-deadline",
            operands: 1,
            ..NONE
        },
    ),
    (
        "taskset",
        Reading {
            flags: "-c --cpu-list",
            operands: 1,
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

/// Where the words of the last wrapper the walk read end, by the layout it gave the command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum End {
    /// At the program, this index.
    Program(usize),
    /// At the word or words the body came from, starting at this index.
    Body(usize),
    /// At the end of the command: the body is inside one of its words (`env -Sbody`,
    /// `--command=body`) or built from several (`alias`).
    Attached,
    /// Not checked: nothing after the last wrapper is a program or a body (`find`, a script
    /// file, a shell reading stdin, no command).
    Unchecked,
}

/// How the words of one wrapper must end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Exactly at the next wrapper or the program.
    Next,
    /// With an option of [`Reading::body`] (or anywhere, when the wrapper has none), the
    /// body being the next word.
    Body,
    /// At a word holding the body after an option of [`Reading::body`] (or anywhere, when the
    /// wrapper has none).
    Attached,
}

/// The index of the first word of `words` from which the walk's placing of the program or
/// body is not certain, or `None` when every wrapper's words read with certainty. `roles` are
/// the walk's, index for index; `end` says where the last wrapper's words end.
pub(super) fn first_uncertain(words: &[Word], roles: &[Role], end: End) -> Option<usize> {
    let wrappers: Vec<usize> = roles
        .iter()
        .enumerate()
        .filter(|(_, role)| **role == Role::Wrapper)
        .map(|(at, _)| at)
        .collect();
    for (nth, &at) in wrappers.iter().enumerate() {
        let (stop, mode) = match (wrappers.get(nth.saturating_add(1)), end) {
            (Some(&next), _) => (next, Mode::Next),
            (None, End::Program(program)) => (program, Mode::Next),
            (None, End::Body(body)) => (body, Mode::Body),
            (None, End::Attached) => (words.len(), Mode::Attached),
            (None, End::Unchecked) => return None,
        };
        let start = at.saturating_add(1);
        let reading = words
            .get(at)
            .and_then(denylist::program_name)
            .and_then(reading)
            .unwrap_or(NONE);
        // Bounds the walk did not give in order are no certain reading.
        let (Some(run_words), Some(run_roles)) = (words.get(start..stop), roles.get(start..stop))
        else {
            return Some(start);
        };
        let run = Run {
            words: run_words,
            roles: run_roles,
        };
        if let Some(offset) = run.first_uncertain(reading, mode) {
            return Some(start.saturating_add(offset));
        }
    }
    None
}

/// The [`Reading`] of the wrapper `name`; `None` for one the tables do not know.
fn reading(name: &str) -> Option<Reading> {
    READINGS
        .iter()
        .find(|(wrapper, _)| *wrapper == name)
        .map(|&(_, reading)| reading)
}

/// Whether the space-separated `list` holds `text`.
fn holds(list: &str, text: &str) -> bool {
    list.split_ascii_whitespace().any(|item| item == text)
}

/// What an option word is to a wrapper, by its [`Reading`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Opt {
    /// An exact flag.
    Flag,
    /// An exact value option: its value is the next word.
    Value,
    /// A value option with its value in the same word (`-uroot`, `--user=root`).
    Attached,
    /// An exact body option: the body is the next word.
    Body,
    /// A body option with the body in the same word (`-Sbody`, `--command=body`).
    AttachedBody,
    /// Anything else: an option the tables do not know, or a cluster (`-iu`).
    Unknown,
}

impl Reading {
    /// What the option word `text` is to this wrapper.
    fn option(self, text: &str) -> Opt {
        if holds(self.flags, text) {
            return Opt::Flag;
        }
        if holds(self.values, text) {
            return Opt::Value;
        }
        if holds(self.body, text) {
            return Opt::Body;
        }
        // The option a value is attached to: `--name` before `=`, or `-` and the first letter.
        let named = match text.split_once('=') {
            Some((name, _)) if name.starts_with("--") => Some(name),
            Some(_) => None,
            None if text.starts_with("--") => None,
            None => text
                .char_indices()
                .nth(2)
                .and_then(|(second, _)| text.get(..second)),
        };
        match named {
            Some(name) if holds(self.values, name) => Opt::Attached,
            Some(name) if holds(self.body, name) => Opt::AttachedBody,
            _ => Opt::Unknown,
        }
    }
}

/// The words between a wrapper and what it runs, with the walk's role for each.
#[derive(Debug, Clone, Copy)]
struct Run<'w> {
    words: &'w [Word],
    roles: &'w [Role],
}

impl<'w> Run<'w> {
    /// The word at `at` when the walk gave it no role of its own (an option, a value or an
    /// operand); `None` past the end or for a keyword, assignment or wrapper.
    fn other(self, at: usize) -> Option<&'w Word> {
        match self.roles.get(at) {
            Some(Role::Other) => self.words.get(at),
            _ => None,
        }
    }

    /// The offset of the first word `reading` does not read with certainty, or `None` when
    /// every word does and the run ends as `mode` requires.
    fn first_uncertain(self, reading: Reading, mode: Mode) -> Option<usize> {
        let len = self.words.len();
        let mut at = 0_usize;
        let mut operands = 0_usize;
        let mut options = true;
        while let Some(word) = self.other(at) {
            let text = word.text.as_str();
            let next = at.saturating_add(1);
            if options && word.is_literal() && text == "--" {
                options = false;
                at = next;
                continue;
            }
            let is_option = options
                && word.is_literal()
                && text.starts_with('-')
                && (text != "-" || holds(reading.flags, text));
            if !is_option {
                if operands < reading.operands {
                    operands = operands.saturating_add(1);
                } else if !reading.operands_anywhere {
                    break;
                }
                at = next;
                continue;
            }
            match reading.option(text) {
                Opt::Flag | Opt::Attached => at = next,
                Opt::Value if self.other(next).is_some() => at = next.saturating_add(1),
                Opt::Body if mode == Mode::Body && next == len => return None,
                Opt::AttachedBody if mode == Mode::Attached => return None,
                Opt::Value | Opt::Body | Opt::AttachedBody | Opt::Unknown => return Some(at),
            }
        }
        if operands < reading.operands {
            return Some(at);
        }
        if reading.assignments {
            while self.roles.get(at) == Some(&Role::Assignment) {
                at = at.saturating_add(1);
            }
        }
        // A wrapper with body options runs a body only through one of them.
        let unended = mode != Mode::Next && !reading.body.is_empty();
        (at < len || unended).then_some(at)
    }
}

#[cfg(test)]
mod tests;
