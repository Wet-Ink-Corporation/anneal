//! Unwrapping a simple command down to the program that really runs.
//!
//! The command strings wrappers run (`bash -c`, `su -c`, `trap`, `watch`, ...) and `find`'s
//! actions are read in [`strings`].

mod strings;

use std::borrow::Cow;
use std::ops::Range;
use std::ptr;

use crate::hook::guard::shell::{Shape, Word};

/// Words that may start a simple command before its program.
const KEYWORDS: [&str; 9] = [
    "!", "{", "if", "then", "else", "elif", "do", "while", "until",
];

/// Shells whose `-c` string is a command, and which read a script from stdin without one.
const SHELLS: [&str; 7] = ["bash", "sh", "zsh", "dash", "ksh", "mksh", "ash"];

/// Programs the guard checks, or that run shell text: the arguments of a program it does not
/// know are read as a command from each word naming one of these (or a wrapper).
const CHECKED: [&str; 16] = [
    "rm", "git", "tee", "truncate", "cp", "mv", "install", "ln", "link", "sed", "unlink", "rmdir",
    "shred", "chmod", "chown", "chgrp",
];

/// Programs and keywords that only print, search, test or describe their arguments.
const PASSIVE: [&str; 20] = [
    "echo", "printf", "grep", "egrep", "fgrep", "rg", "man", "info", "help", "which", "whereis",
    "type", "whatis", "apropos", "for", "select", "case", "test", "[", "[[",
];

/// Options that take a value in the next word, per wrapper. `sudo` and `doas` share theirs
/// (`doas -a style`, `-C config`, `-u user`). sudo's `-h` takes a host only when a word that
/// is not an option follows (else it is `--help`, which runs nothing), so the next word is
/// always read as its value. `--login-class` (`-c`) is left out: the flag `--login` would
/// read as its abbreviation; `--login-class=class` keeps its value in its own word.
const SUDO_VALUES: &[&str] = &[
    "-a",
    "-c",
    "-u",
    "-g",
    "-h",
    "-p",
    "-C",
    "-D",
    "-R",
    "-r",
    "-t",
    "-T",
    "-U",
    "--auth-type",
    "--user",
    "--group",
    "--host",
    "--prompt",
    "--chdir",
    "--chroot",
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
/// `--eof`, `--replace` and `--max-lines` are absent: their value is optional and only ever
/// follows `=`, so the next word is the command. `-J`, `-R` and `-S` are BSD xargs's.
const XARGS_VALUES: &[&str] = &[
    "-a",
    "-d",
    "-E",
    "-I",
    "-J",
    "-L",
    "-n",
    "-P",
    "-R",
    "-s",
    "-S",
    "--arg-file",
    "--delimiter",
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

/// Where [`unwrap`] finds what a simple command runs, by word position: the view the guard's
/// redaction (`guard::redact`) renders, so it reads wrappers exactly as the denylist does.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::hook::guard) enum Layout<'a> {
    /// The program is the word at this index: the words before it are keywords, assignments
    /// and wrappers with their options and values, the words after it its arguments.
    Program(usize),
    /// Shell text the wrappers run: a `-c` string, `eval`'s or `watch`'s words joined,
    /// `env -S`, `trap`'s action, an alias's value.
    Script(Cow<'a, str>),
    /// `find` with actions (`-exec ... ;`).
    Exec(FindActions),
    /// No program to read further: only keywords and assignments, a shell reading its
    /// script from standard input or a file, or no command at all.
    Other,
}

/// Where `find` and its actions are in a simple command, by word position.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::hook::guard) struct FindActions {
    /// The index of the word naming `find`, the last wrapper the walk reads.
    pub(in crate::hook::guard) find: usize,
    /// The words of each action, by index range, in order.
    pub(in crate::hook::guard) actions: Vec<Range<usize>>,
}

/// How the words of one simple command are laid out once wrappers are read ([`unwrap`]), and
/// the role the walk gives each word, by index ([`roles`]).
pub(in crate::hook::guard) fn layout(words: &[Word]) -> (Layout<'_>, Vec<Role>) {
    let roles = roles(words);
    let layout = match unwrap(words) {
        // `unwrap` only ever drops words from the front, so the arguments end `words`.
        Run::Command { args, .. } => words
            .len()
            .checked_sub(args.len())
            .and_then(|end| end.checked_sub(1))
            .map_or(Layout::Other, Layout::Program),
        Run::Script(text) => Layout::Script(text),
        // `find` peeled last, ending the walk with its actions.
        Run::Exec(actions) => roles
            .iter()
            .rposition(|role| *role == Role::Wrapper)
            .map_or(Layout::Other, |find| {
                Layout::Exec(FindActions {
                    find,
                    actions: actions
                        .into_iter()
                        .filter_map(|action| range_in(words, action))
                        .collect(),
                })
            }),
        Run::Nothing | Run::StdinScript(_) | Run::Unseen(_) => Layout::Other,
    };
    (layout, roles)
}

/// The part a word of a simple command plays as [`unwrap`] reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::hook::guard) enum Role {
    /// A keyword before the program (`if`, `{`, `!`).
    Keyword,
    /// `NAME=value` before the program, or after a wrapper that runs a command
    /// (`sudo K=v cmd`). An argument shaped like one (`find -name x=y`) is not.
    Assignment,
    /// A word naming a wrapper the walk removes (`sudo`, `env`, a shell, `find`, ...), in
    /// the place of a program.
    Wrapper,
    /// Anything else: the program, an option, a value or an operand.
    Other,
}

/// The role [`unwrap`]'s walk gives each word of `words`, by index: a word is a wrapper only
/// where it stands in the place of a program, never as a value or an operand
/// (`sudo -D /x/nice rm`, `find /a/env -delete`).
fn roles(words: &[Word]) -> Vec<Role> {
    let mut roles = vec![Role::Other; words.len()];
    let mut rest = words;
    loop {
        let skipped = skip_prefix(rest);
        let prefix = rest.len().saturating_sub(skipped.len());
        let at = words.len().saturating_sub(rest.len());
        for (word, slot) in rest.iter().take(prefix).zip(roles.iter_mut().skip(at)) {
            // `skip_prefix` skips only keywords and assignments.
            *slot = if is_keyword(word) {
                Role::Keyword
            } else {
                Role::Assignment
            };
        }
        let [first, tail @ ..] = skipped else {
            return roles;
        };
        let Program::Named(name) = program(first) else {
            return roles;
        };
        let peeled = peel(name, tail);
        if matches!(peeled, Peel::Program) {
            return roles;
        }
        if let Some(slot) = roles.get_mut(words.len().saturating_sub(skipped.len())) {
            *slot = Role::Wrapper;
        }
        rest = match peeled {
            Peel::Inner(inner) | Peel::Appending(inner) => inner,
            Peel::Done(_) | Peel::Program => return roles,
        };
    }
}

/// Where `part`, a non-empty run of the words of `words`, lies in it.
fn range_in(words: &[Word], part: &[Word]) -> Option<Range<usize>> {
    let first = part.first()?;
    let start = words.iter().position(|word| ptr::eq(word, first))?;
    Some(start..start.saturating_add(part.len()))
}

/// The program a literal `word` names, without a path or `.exe`; `None` for a word that is
/// not literal.
pub(in crate::hook::guard) fn program_name(word: &Word) -> Option<&str> {
    match program(word) {
        Program::Named(name) => Some(name),
        Program::Unknown(_) => None,
    }
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

/// How a wrapper of `words` changes the working directory before the command it wraps runs
/// (`env --chdir`, `sudo --chdir`, `find -execdir`), walking the wrappers as [`unwrap`] does.
pub(super) fn chdir(words: &[Word]) -> Option<&'static str> {
    walk(words, strings::chdir)
}

/// Whether `words` define a function, an alias or a path for a command name:
/// `function NAME ...`, `alias ...` or `hash -p PATH NAME`, under any wrapper [`unwrap`]
/// removes (`builtin alias`, `command hash -p`), or a word that exports a function to a
/// child bash (`BASH_FUNC_NAME%%=...`).
pub(super) fn defines(words: &[Word]) -> bool {
    if words.iter().any(|word| word.text.contains("BASH_FUNC_")) {
        return true;
    }
    walk(words, |name, rest| {
        let hash_path = name == "hash" && rest.iter().any(sets_hash_path);
        (matches!(name, "function" | "alias") || hash_path).then_some(())
    })
    .is_some()
}

/// Whether `word` is a `hash` option cluster holding `-p` (`-p`, `-rp`).
fn sets_hash_path(word: &Word) -> bool {
    word.text
        .strip_prefix('-')
        .is_some_and(|cluster| !cluster.starts_with('-') && cluster.contains('p'))
}

/// The first answer `visit` gives for a program of `words` and its arguments, walking the
/// wrappers as [`unwrap`] does (leading assignments and keywords, then each wrapper peeled);
/// `None` at a program that is not a literal name or a command that wraps nothing more.
fn walk<'w, T>(
    words: &'w [Word],
    mut visit: impl FnMut(&'w str, &'w [Word]) -> Option<T>,
) -> Option<T> {
    let mut words = skip_prefix(words);
    loop {
        let [first, rest @ ..] = words else {
            return None;
        };
        let Program::Named(name) = program(first) else {
            return None;
        };
        if let Some(found) = visit(name, rest) {
            return Some(found);
        }
        words = match peel(name, rest) {
            Peel::Inner(inner) | Peel::Appending(inner) => skip_prefix(inner),
            Peel::Done(_) | Peel::Program => return None,
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
        "chrt" => pid_or_command(rest, CHRT_VALUES),
        "taskset" => pid_or_command(rest, &[]),
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

/// `chrt` and `taskset`: the command after their options and the priority or CPU mask, or
/// nothing when an option is `-p` (`--pid`), a flag (util-linux's `+...p...` optstrings) that
/// makes them act on the running process whose pid ends the words and run no command.
fn pid_or_command<'a>(words: &'a [Word], with_value: &[&str]) -> Peel<'a> {
    // The walk of `skip_options`, looking at each option word but not at the values.
    let mut rest = words;
    while let [first, tail @ ..] = rest {
        let text = first.text.as_str();
        if text == "--" {
            rest = tail;
            break;
        }
        if !is_option(first) {
            break;
        }
        if is_long(text, "--pid") || cluster_holds(text, 'p', with_value) {
            return Peel::Done(Run::Nothing);
        }
        rest = if takes_value(text, with_value) {
            skip_one(tail)
        } else {
            tail
        };
    }
    Peel::Inner(skip_one(rest))
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

/// Whether `word` names a program the guard checks, a shell or a wrapper it unwraps.
pub(super) fn names_checked(word: &Word) -> bool {
    match program(word) {
        // Every wrapper peels something off even with no arguments; other programs do not.
        Program::Named(name) => {
            CHECKED.contains(&name) || !matches!(peel(name, &[]), Peel::Program)
        }
        Program::Unknown(_) => false,
    }
}

/// Whether `word` names the program `name` (by path or with `.exe` too).
pub(super) fn names_program(word: &Word, name: &str) -> bool {
    program(word) == Program::Named(name)
}

/// Whether `name`, a program that is not a wrapper the guard unwraps, may run its arguments as
/// a command: it is not one the guard checks, nor one that only prints or searches them.
pub(super) fn may_run_arguments(name: &str) -> bool {
    !(CHECKED.contains(&name) || PASSIVE.contains(&name) || matches!(name, "cd" | "pushd"))
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

/// Whether `word` is an unquoted keyword that may start a simple command (`if`, `{`, `!`).
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

/// A long option as written, `--name` or `--name=value`. `getopt_long` and git accept a long
/// option's name abbreviated to any unique prefix (`--forc` is `--force`).
#[derive(Debug, Clone, Copy)]
pub(super) struct Long<'a> {
    /// The name as written, `--` included.
    name: &'a str,
    /// The value after `=`, if any.
    pub(super) value: Option<&'a str>,
}

impl<'a> Long<'a> {
    /// `text` as a long option: `--` and at least one more character before any `=`.
    pub(super) fn parse(text: &'a str) -> Option<Self> {
        let (name, value) = match text.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (text, None),
        };
        (name.len() > 2 && name.starts_with("--")).then_some(Self { name, value })
    }

    /// Whether this may be the option `full` (`--force`), written out or abbreviated. A
    /// prefix several options share counts as each of them: the guard reads it as the one it
    /// denies, and the program refuses it anyway.
    pub(super) fn is(self, full: &str) -> bool {
        full.starts_with(self.name)
    }
}

/// Whether `text` is the long option `full`, written out or abbreviated, with or without a
/// `=value`.
pub(super) fn is_long(text: &str, full: &str) -> bool {
    Long::parse(text).is_some_and(|long| long.is(full))
}

/// Whether the option `text` takes the next word as its value: a long one of `with_value`
/// (possibly abbreviated) with no `=value` of its own, or a short cluster read as getopt
/// reads it, whose first letter that takes a value (`-u` in `with_value`) is its last
/// (`-iu`); one with more after it has that as its value (`-uroot`, `-iuroot`).
pub(super) fn takes_value(text: &str, with_value: &[&str]) -> bool {
    match Long::parse(text) {
        Some(long) => long.value.is_none() && with_value.iter().any(|option| long.is(option)),
        None => short_cluster(text).is_some_and(|letters| {
            letters
                .char_indices()
                .find(|&(_, letter)| short_takes_value(letter, with_value))
                .is_some_and(|(at, letter)| at.saturating_add(letter.len_utf8()) == letters.len())
        }),
    }
}

/// The letters of a short option cluster (`-iu` is `iu`); `None` for a long option or a word
/// that is not an option.
fn short_cluster(text: &str) -> Option<&str> {
    text.strip_prefix('-')
        .filter(|letters| !letters.is_empty() && !letters.starts_with('-'))
}

/// Whether the short option `-letter` is one of `with_value`.
fn short_takes_value(letter: char, with_value: &[&str]) -> bool {
    with_value.iter().any(|option| {
        option
            .strip_prefix('-')
            .is_some_and(|name| name.chars().eq([letter]))
    })
}

/// Whether a short cluster holds the flag `-letter` before any letter that takes a value
/// (whose value the rest of the cluster is): `-fp` holds `p`, `-Tp` does not when `-T` takes
/// a value.
fn cluster_holds(text: &str, letter: char, with_value: &[&str]) -> bool {
    short_cluster(text).is_some_and(|letters| {
        letters
            .chars()
            .take_while(|&c| !short_takes_value(c, with_value))
            .any(|c| c == letter)
    })
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
        rest = if takes_value(&first.text, with_value) {
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
