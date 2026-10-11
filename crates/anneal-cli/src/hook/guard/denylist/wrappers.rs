//! Unwrapping a simple command down to the program that really runs.
//!
//! The command strings wrappers run (`bash -c`, `su -c`, `trap`, `watch`, ...) and `find`'s
//! actions are read in [`strings`].

mod strings;

use std::borrow::Cow;

use crate::hook::guard::shell::{Shape, Word};

/// Words that may start a simple command before its program.
const KEYWORDS: [&str; 9] = [
    "!", "{", "if", "then", "else", "elif", "do", "while", "until",
];

/// Shells whose `-c` string is a command, and which read a script from stdin without one.
const SHELLS: [&str; 7] = ["bash", "sh", "zsh", "dash", "ksh", "mksh", "ash"];

/// Options that take a value in the next word, per wrapper.
const SUDO_VALUES: &[&str] = &[
    "-u",
    "-g",
    "-h",
    "-p",
    "-C",
    "-D",
    "-r",
    "-t",
    "-T",
    "-U",
    "--user",
    "--group",
    "--host",
    "--prompt",
    "--chdir",
    "--role",
    "--type",
    "--close-from",
    "--other-user",
    "--command-timeout",
];
const EXEC_VALUES: &[&str] = &["-a"];
const TIME_VALUES: &[&str] = &["-f", "-o", "--format", "--output"];
const NICE_VALUES: &[&str] = &["-n", "--adjustment"];
const TIMEOUT_VALUES: &[&str] = &["-s", "-k", "--signal", "--kill-after"];
const STDBUF_VALUES: &[&str] = &["-i", "-o", "-e", "--input", "--output", "--error"];
const IONICE_VALUES: &[&str] = &[
    "-c",
    "-n",
    "-p",
    "-P",
    "-u",
    "--class",
    "--classdata",
    "--pid",
    "--pgid",
    "--uid",
];
const CHRT_VALUES: &[&str] = &[
    "-T",
    "-P",
    "-D",
    "--sched-runtime",
    "--sched-period",
    "--sched-deadline",
];
const XARGS_VALUES: &[&str] = &[
    "-a",
    "-d",
    "-E",
    "-I",
    "-L",
    "-n",
    "-P",
    "-s",
    "--arg-file",
    "--delimiter",
    "--eof",
    "--replace",
    "--max-lines",
    "--max-args",
    "--max-procs",
    "--max-chars",
    "--process-slot-var",
];

/// The program of a simple command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Program<'a> {
    /// Its name, without a path or `.exe`.
    Named(&'a str),
    /// A word that is not a literal name (`$(echo rm)`, `"$GIT"`, a glob or a brace).
    Unknown(&'a Word),
}

/// What a simple command runs, once wrappers are removed.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Run<'a> {
    /// No program (only assignments or keywords).
    Nothing,
    /// A program and its arguments; `appended` when `xargs` adds arguments the guard cannot see.
    Command {
        /// The program.
        program: Program<'a>,
        /// Its arguments.
        args: &'a [Word],
        /// Whether more arguments are appended at run time.
        appended: bool,
    },
    /// Shell text to read in turn: a `-c` string, `eval`'s arguments, `env -S`, `trap`.
    Script(Cow<'a, str>),
    /// A shell (named) reading its script from standard input.
    StdinScript(&'a str),
    /// A shell (named) running a script from a process substitution or a file descriptor,
    /// known only at run time.
    Unseen(&'a str),
    /// The commands `find`'s actions run, each with matches appended.
    Exec(Vec<&'a [Word]>),
}

/// One wrapper removed.
enum Peel<'a> {
    /// The wrapped command's words.
    Inner(&'a [Word]),
    /// The wrapped command's words, with arguments appended at run time (`xargs`).
    Appending(&'a [Word]),
    /// The wrapper runs this.
    Done(Run<'a>),
    /// Not a wrapper: the program itself.
    Program,
}

/// Strips keywords, assignments and wrappers from `words`.
pub(super) fn unwrap(words: &[Word]) -> Run<'_> {
    let mut words = skip_prefix(words);
    let mut appended = false;
    loop {
        let [first, rest @ ..] = words else {
            return Run::Nothing;
        };
        let program = program(first);
        let peeled = match program {
            Program::Named(name) => peel(name, rest),
            Program::Unknown(_) => Peel::Program,
        };
        words = match peeled {
            Peel::Inner(inner) => skip_prefix(inner),
            Peel::Appending(inner) => {
                appended = true;
                skip_prefix(inner)
            }
            Peel::Done(run) => return run,
            Peel::Program => {
                return Run::Command {
                    program,
                    args: rest,
                    appended,
                };
            }
        };
    }
}

/// Removes the wrapper `name` from its arguments `rest`.
fn peel<'a>(name: &'a str, rest: &'a [Word]) -> Peel<'a> {
    match name {
        "sudo" | "doas" => strings::sudo(rest),
        "env" => strings::env(rest),
        "command" | "nohup" | "builtin" | "setsid" | "unbuffer" => {
            Peel::Inner(skip_options(rest, &[]))
        }
        "exec" => Peel::Inner(skip_options(rest, EXEC_VALUES)),
        "time" => Peel::Inner(skip_options(rest, TIME_VALUES)),
        "nice" => Peel::Inner(skip_options(rest, NICE_VALUES)),
        "stdbuf" => Peel::Inner(skip_options(rest, STDBUF_VALUES)),
        "ionice" => Peel::Inner(skip_options(rest, IONICE_VALUES)),
        // The duration, the priority, the CPU mask, the function's name.
        "timeout" => Peel::Inner(skip_one(skip_options(rest, TIMEOUT_VALUES))),
        "chrt" => Peel::Inner(skip_one(skip_options(rest, CHRT_VALUES))),
        "taskset" => Peel::Inner(skip_one(skip_options(rest, &[]))),
        "function" => Peel::Inner(skip_one(rest)),
        // `busybox <applet> ...`.
        "busybox" => Peel::Inner(rest),
        "coproc" => strings::coproc(rest),
        "flock" => strings::flock(rest),
        "xargs" => Peel::Appending(skip_options(rest, XARGS_VALUES)),
        "eval" => Peel::Done(Run::Script(Cow::Owned(join(rest)))),
        "alias" => strings::alias(rest),
        "source" | "." => strings::source(rest),
        "trap" => strings::trap(rest),
        "su" => strings::su(rest),
        "script" => strings::script(rest),
        "watch" => strings::watch(rest),
        "find" => strings::find(rest),
        shell if SHELLS.contains(&shell) => strings::shell(shell, rest),
        _ => Peel::Program,
    }
}

/// The program `word` names: its last path component without `.exe`, or unknown when the
/// word is not literal.
fn program(word: &Word) -> Program<'_> {
    if !word.is_literal() {
        return Program::Unknown(word);
    }
    let text = word.text.as_str();
    let name = text.rsplit(['/', '\\']).next().unwrap_or(text);
    Program::Named(name.strip_suffix(".exe").unwrap_or(name))
}

/// Skips leading keywords (`if`, `then`, `{`, `!`, ...) and `NAME=value` assignments.
fn skip_prefix(words: &[Word]) -> &[Word] {
    let mut rest = words;
    while let [first, tail @ ..] = rest {
        if !(is_keyword(first) || is_assignment(first)) {
            break;
        }
        rest = tail;
    }
    rest
}

/// Skips leading keywords (`if`, `then`, `{`, `!`, ...).
pub(super) fn skip_keywords(words: &[Word]) -> &[Word] {
    let at = words.iter().take_while(|word| is_keyword(word)).count();
    words.split_at_checked(at).map_or(words, |(_, rest)| rest)
}

fn is_keyword(word: &Word) -> bool {
    !word.quoted && KEYWORDS.contains(&word.text.as_str())
}

/// `NAME=value`, with `NAME` a shell identifier.
pub(super) fn is_assignment(word: &Word) -> bool {
    word.text.split_once('=').is_some_and(|(name, _)| {
        name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Whether `word` is an option: literal, starting with `-`, and not `-` alone.
pub(super) fn is_option(word: &Word) -> bool {
    word.is_literal() && may_be_option(word)
}

/// Whether `word` may be an option once expanded: it starts with `-` (not `-` alone) and
/// holds no variable. A brace-expanded `-f` (a pattern) counts.
pub(super) fn may_be_option(word: &Word) -> bool {
    word.shape != Shape::Dynamic && word.text.starts_with('-') && word.text != "-"
}

/// Skips options up to the first operand, and a `--` that ends them. Options in `with_value`
/// take the next word as their value.
pub(super) fn skip_options<'a>(words: &'a [Word], with_value: &[&str]) -> &'a [Word] {
    let mut rest = words;
    while let [first, tail @ ..] = rest {
        if first.text == "--" {
            return tail;
        }
        if !is_option(first) {
            break;
        }
        rest = if with_value.contains(&first.text.as_str()) {
            skip_one(tail)
        } else {
            tail
        };
    }
    rest
}

/// `words` without its first word.
pub(super) fn skip_one(words: &[Word]) -> &[Word] {
    words.split_first().map_or(words, |(_, rest)| rest)
}

/// The words' texts joined by spaces, as `eval` joins its arguments.
pub(super) fn join(words: &[Word]) -> String {
    words
        .iter()
        .map(|word| word.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}
