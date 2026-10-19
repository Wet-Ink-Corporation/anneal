//! Unwrapping a simple command down to the program that really runs.
//!
//! The command strings wrappers run (`bash -c`, `su -c`, `trap`, `watch`, ...) and `find`'s
//! actions are read in [`strings`].

mod grammar;
mod strings;
mod tables;

use std::borrow::Cow;
use std::ops::Range;
use std::ptr;

use super::braces;
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

/// Options that take a value in the next word, for bash's `exec` and `time`; every other
/// wrapper's options are read by its [`grammar::Grammar`] in [`tables`].
const EXEC_VALUES: &[&str] = &["-a"];
const TIME_VALUES: &[&str] = &["-f", "-o", "--format", "--output"];

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
    word.is_literal().then(|| name_of(&word.text))
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
        "nice" => Peel::Inner(tables::NICE.parse(rest).rest),
        "stdbuf" => Peel::Inner(tables::STDBUF.parse(rest).rest),
        "ionice" => Peel::Inner(tables::IONICE.parse(rest).rest),
        // The duration, the CPU mask, the function's name.
        "timeout" => Peel::Inner(skip_one(tables::TIMEOUT.parse(rest).rest)),
        "chrt" => Peel::Inner(chrt(rest)),
        "taskset" => Peel::Inner(skip_one(tables::TASKSET.parse(rest).rest)),
        "function" => Peel::Inner(skip_one(rest)),
        // `busybox <applet> ...`.
        "busybox" => Peel::Inner(rest),
        "coproc" => strings::coproc(rest),
        "flock" => strings::flock(rest),
        "xargs" => Peel::Appending(tables::XARGS.parse(rest).rest),
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

/// `chrt [options] [PRIORITY] command`: util-linux 2.42 reads a priority only when it is all
/// digits (or must, for `-f` and `-r`, refusing anything else), then skips one `--`; 2.39
/// always reads one, refusing one not all digits. With `-p` the words after the priority are
/// a pid for both, yet chrt runs them when the last is 0 (or -1), so they are read as a command.
fn chrt(words: &[Word]) -> &[Word] {
    let rest = tables::CHRT.parse(words).rest;
    let rest = match rest {
        [priority, tail @ ..]
            if priority.is_literal()
                && !priority.text.is_empty()
                && priority.text.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            tail
        }
        _ => rest,
    };
    match rest {
        [dashes, tail @ ..] if dashes.text == "--" => tail,
        _ => rest,
    }
}

/// The program `word` names: its last path component without `.exe`, or unknown when the
/// word is not read as written ([`braces::reads_as_written`]: literal, or a name brace
/// expansion made, `rm` from `{rm,-rf,/}`).
fn program(word: &Word) -> Program<'_> {
    if !braces::reads_as_written(word) {
        return Program::Unknown(word);
    }
    Program::Named(name_of(&word.text))
}

/// The program `text` names: its last path component, without `.exe`.
fn name_of(text: &str) -> &str {
    let name = text.rsplit(['/', '\\']).next().unwrap_or(text);
    name.strip_suffix(".exe").unwrap_or(name)
}

/// Whether `word` names a program the guard checks, a shell or a wrapper it unwraps.
pub(super) fn names_checked(word: &Word) -> bool {
    match program(word) {
        Program::Named(name) => is_checked(name),
        Program::Unknown(_) => false,
    }
}

/// Whether `name` is a program the guard checks, a shell or a wrapper it unwraps.
fn is_checked(name: &str) -> bool {
    // Every wrapper peels something off even with no arguments; other programs do not.
    CHECKED.contains(&name) || !matches!(peel(name, &[]), Peel::Program)
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

/// Whether `word` is an option: read as written ([`braces::reads_as_written`]: literal, or
/// `-u` from `{sudo,-u,root,..}`), starting with `-`, and not `-` alone.
pub(super) fn is_option(word: &Word) -> bool {
    braces::reads_as_written(word) && may_be_option(word)
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

/// Whether the option `text` takes the next word as its value: it is one of `with_value`
/// (a long one possibly abbreviated) with no `=value` of its own.
pub(super) fn takes_value(text: &str, with_value: &[&str]) -> bool {
    match Long::parse(text) {
        Some(long) => long.value.is_none() && with_value.iter().any(|option| long.is(option)),
        None => with_value.contains(&text),
    }
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
