//! Wrappers that run a command string or read a script: shells, `sudo -s`, `env -S`, `su`,
//! `script`, `trap`, `watch`, `flock -c`, and `find`'s actions.

use std::borrow::Cow;

use super::grammar::{Grammar, Name, Read};
use super::tables::{ENV, FLOCK, SCRIPT, SU, SU_TO_USER, SUDO, WATCH};
use super::{Long, Peel, Run, is_option, join, skip_one, skip_options};
use crate::hook::guard::denylist::braces::reads_as_written;
use crate::hook::guard::shell::Word;

const SHELL_VALUES: &[&str] = &["--rcfile", "--init-file"];

/// The `find` actions that run a command on each match.
const FIND_EXEC: [&str; 4] = ["-exec", "-execdir", "-ok", "-okdir"];

/// How the wrapper `name`, given `words`, changes the working directory before its command
/// runs: `env -C DIR` (`--chdir`), `sudo -D DIR` (`--chdir`), a login shell started in the
/// user's home (`sudo -i`/`--login`, `su -`/`-l`/`--login`), or a `find` action run from each
/// match's directory (`-execdir`, `-okdir`).
pub(super) fn chdir(name: &str, words: &[Word]) -> Option<&'static str> {
    match name {
        "env" => options_hold(words, &ENV, 'C', "chdir").then_some("env --chdir"),
        "sudo" => options_hold(words, &SUDO, 'D', "chdir")
            .then_some("sudo --chdir")
            .or_else(|| options_hold(words, &SUDO, 'i', "login").then_some("sudo --login")),
        "su" => su_login(words).then_some("su --login"),
        "find" => words
            .iter()
            .any(|word| matches!(word.text.as_str(), "-execdir" | "-okdir"))
            .then_some("find -execdir"),
        _ => None,
    }
}

/// Whether the options leading `words`, read by `grammar`, hold the short letter `short` or the
/// long option `long`. A word among them that starts with `-` and holds the letter counts too,
/// as another option's value or as `env -S`'s string, which env reads as its own options: the
/// guard reads it as the change.
fn options_hold(words: &[Word], grammar: &Grammar, short: char, long: &str) -> bool {
    let read = grammar.parse(words);
    let options = words.len().saturating_sub(read.rest.len());
    read.holds(short, long)
        || words.iter().take(options).any(|word| {
            is_option(word) && !word.text.starts_with("--") && word.text.contains(short)
        })
}

/// `sudo` and `doas`: the command after their options, or, with `-s` or `-i` and no command,
/// a shell reading stdin.
pub(super) fn sudo(words: &[Word]) -> Peel<'_> {
    let read = SUDO.parse(words);
    if read.rest.is_empty() && (read.holds('s', "shell") || read.holds('i', "login")) {
        Peel::Done(Run::StdinScript("sudo"))
    } else {
        Peel::Inner(read.rest)
    }
}

/// `env`'s command after its options and a lone `-` (`-i`'s old spelling), or the script `-S`
/// (`--split-string`) runs, followed by the words after it.
pub(super) fn env(words: &[Word]) -> Peel<'_> {
    let read = ENV.parse(words);
    if let Some(split) = read.named('S', "split-string").next() {
        // `-S` as the last word splits nothing: an empty script.
        let script = split.value.unwrap_or("");
        return Peel::Done(Run::Script(if split.after.is_empty() {
            Cow::Borrowed(script)
        } else {
            Cow::Owned(format!("{script} {}", join(split.after)))
        }));
    }
    match read.rest {
        [dash, tail @ ..] if dash.text == "-" && reads_as_written(dash) => Peel::Inner(tail),
        rest => Peel::Inner(rest),
    }
}

/// A long option's value (`None` when it is missing) and the words after it.
type Valued<'a> = (Option<&'a str>, &'a [Word]);

/// When `text` is the long option `full` (abbreviated or not): its value, after `=` or in the
/// next word of `tail`, and the words after that.
fn long_value<'a>(text: &'a str, tail: &'a [Word], full: &str) -> Option<Valued<'a>> {
    let long = Long::parse(text).filter(|long| long.is(full))?;
    Some(match long.value {
        Some(value) => (Some(value), tail),
        None => (tail.first().map(|word| word.text.as_str()), skip_one(tail)),
    })
}

/// A wrapper running the command string `command`; nothing when it has none.
fn command_string(command: Option<&str>) -> Peel<'_> {
    command.map_or(Peel::Done(Run::Nothing), |command| {
        Peel::Done(Run::Script(Cow::Borrowed(command)))
    })
}

/// `coproc [NAME] command`: a name only comes before a compound command (`{`).
pub(super) fn coproc(words: &[Word]) -> Peel<'_> {
    match words {
        [_, brace, ..] if !brace.quoted && brace.text == "{" => Peel::Inner(skip_one(words)),
        _ => Peel::Inner(words),
    }
}

/// `flock [options] FILE command` or `flock [options] FILE -c string`.
pub(super) fn flock(words: &[Word]) -> Peel<'_> {
    let after_file = skip_one(FLOCK.parse(words).rest);
    match after_file {
        [flag, tail @ ..] if is_option(flag) && flag.text == "-c" => {
            command_string(tail.first().map(|string| string.text.as_str()))
        }
        [flag, tail @ ..] if is_option(flag) => long_value(&flag.text, tail, "--command")
            .map_or(Peel::Inner(after_file), |(command, _)| {
                command_string(command)
            }),
        _ => Peel::Inner(after_file),
    }
}

/// `trap ACTION SIGNAL...`: the action is shell text.
pub(super) fn trap(words: &[Word]) -> Peel<'_> {
    skip_options(words, &[])
        .first()
        .map_or(Peel::Done(Run::Nothing), |action| {
            Peel::Done(Run::Script(Cow::Borrowed(&action.text)))
        })
}

/// Whether `su [options] [USER]` starts a login shell, which runs from the user's home: `-l`
/// (`--login`), or a `-` among its operands. `su` takes options after the user too.
fn su_login(words: &[Word]) -> bool {
    let read = SU.parse(words);
    let operands_end = words.len().saturating_sub(read.rest.len());
    read.holds('l', "login")
        || words
            .iter()
            .take(operands_end)
            .any(|word| word.text == "-" && reads_as_written(word))
}

/// The command strings of `read`'s `-c` (`--command`, and for `su` `--session-command`)
/// options, in order; `None` for one missing its string.
fn commands(read: Read<'_>) -> impl Iterator<Item = Option<&str>> {
    read.options
        .into_iter()
        .filter(|option| {
            matches!(
                option.name,
                Name::Short('c') | Name::Long("command" | "session-command")
            )
        })
        .map(|option| option.value)
}

/// The command strings the user's shell may read from `su`'s words after its user. `su` hands
/// its operands after the user (after `--` too) to the shell, and with `POSIXLY_CORRECT` set
/// every word after the user, so each short cluster holding `c` among them is read as the
/// shell reads it: taking the next word as its command string.
fn shell_commands(words: &[Word]) -> impl Iterator<Item = Option<&str>> {
    let after_user = skip_one(SU_TO_USER.parse(words).rest);
    after_user
        .iter()
        .enumerate()
        .filter(|(_, word)| {
            is_option(word) && !word.text.starts_with("--") && word.text.contains('c')
        })
        .map(|(at, _)| {
            after_user
                .get(at.saturating_add(1))
                .map(|string| string.text.as_str())
        })
}

/// The command strings `commands` names: the last one runs, the earlier ones are read too. One
/// body is the word or part of a word it was written in; several are joined by newlines.
/// Without one, `wrapper` runs a shell reading stdin; with only a `-c` missing its string,
/// nothing.
fn bodies<'a>(
    commands: impl IntoIterator<Item = Option<&'a str>>,
    wrapper: &'static str,
) -> Peel<'a> {
    let mut any = false;
    let mut found: Vec<&'a str> = Vec::new();
    for command in commands {
        any = true;
        if let Some(body) = command.filter(|body| !found.contains(body)) {
            found.push(body);
        }
    }
    match found.as_slice() {
        [] if any => Peel::Done(Run::Nothing),
        [] => Peel::Done(Run::StdinScript(wrapper)),
        [body] => Peel::Done(Run::Script(Cow::Borrowed(body))),
        several => Peel::Done(Run::Script(Cow::Owned(several.join("\n")))),
    }
}

/// `su [options] [USER] [ARG...]`: its `-c` strings and those its arguments give the user's
/// shell, or, without one, a shell reading stdin.
pub(super) fn su(words: &[Word]) -> Peel<'_> {
    bodies(commands(SU.parse(words)).chain(shell_commands(words)), "su")
}

/// `script [options] [FILE]`: its `-c` strings, or, without one, a shell reading stdin.
pub(super) fn script(words: &[Word]) -> Peel<'_> {
    bodies(commands(SCRIPT.parse(words)), "script")
}

/// `watch [options] command...`: the words joined are run by `sh -c`.
pub(super) fn watch(words: &[Word]) -> Peel<'_> {
    let rest = WATCH.parse(words).rest;
    if rest.is_empty() {
        Peel::Done(Run::Nothing)
    } else {
        Peel::Done(Run::Script(Cow::Owned(join(rest))))
    }
}

/// A shell: its `-c` string, nothing when it runs a script file, or, with `-s` or no operand,
/// a script read from stdin.
pub(super) fn shell<'a>(name: &'a str, words: &'a [Word]) -> Peel<'a> {
    let mut rest = words;
    let (mut command_string, mut from_stdin) = (false, false);
    while let [first, tail @ ..] = rest {
        let text = first.text.as_str();
        if text == "--version" || text == "--help" {
            return Peel::Done(Run::Nothing);
        }
        if text == "--" || text == "-" {
            rest = tail;
            break;
        }
        let option = reads_as_written(first) && (text.starts_with('-') || text.starts_with('+'));
        if !option {
            break;
        }
        let long = text.starts_with("--");
        let short = !long && text.starts_with('-');
        command_string |= short && text.contains('c');
        from_stdin |= short && text.contains('s');
        let takes_value = if long {
            SHELL_VALUES.contains(&text)
        } else {
            text.ends_with(['o', 'O'])
        };
        rest = if takes_value { skip_one(tail) } else { tail };
    }
    if command_string {
        return rest.first().map_or(Peel::Done(Run::Nothing), |string| {
            Peel::Done(Run::Script(Cow::Borrowed(&string.text)))
        });
    }
    // With `-s` the operands are arguments and the script is stdin.
    match rest.first().filter(|_| !from_stdin) {
        None => Peel::Done(Run::StdinScript(name)),
        Some(script) => script_file(name, script),
    }
}

/// The paths through which a script file is standard input.
const STDIN_PATHS: [&str; 3] = ["/dev/stdin", "/dev/fd/0", "/proc/self/fd/0"];

/// `shell` (or `source`) running the script file `script`: stdin when it names it, unseen
/// when it is a process substitution, else a file the guard does not read.
fn script_file<'a>(shell: &'a str, script: &Word) -> Peel<'a> {
    let path = script.text.as_str();
    let descriptor = path.starts_with("/dev/fd/") || path.starts_with("/proc/");
    if reads_as_written(script) && STDIN_PATHS.contains(&path) {
        Peel::Done(Run::StdinScript(shell))
    } else if script.is_process_substitution() || descriptor {
        Peel::Done(Run::Unseen(shell))
    } else {
        Peel::Done(Run::Nothing)
    }
}

/// `source FILE` (or `. FILE`).
pub(super) fn source(words: &[Word]) -> Peel<'_> {
    skip_options(words, &[])
        .first()
        .map_or(Peel::Done(Run::Nothing), |script| {
            script_file("source", script)
        })
}

/// `alias NAME=VALUE...`: each value is shell text, run where the alias is used.
pub(super) fn alias(words: &[Word]) -> Peel<'_> {
    let values: Vec<&str> = words
        .iter()
        .filter_map(|word| word.text.split_once('=').map(|(_, value)| value))
        .collect();
    if values.is_empty() {
        Peel::Done(Run::Nothing)
    } else {
        Peel::Done(Run::Script(Cow::Owned(values.join("\n"))))
    }
}

/// `find`: the command of every `-exec`, `-execdir`, `-ok` and `-okdir`, each up to its `;`
/// or `+`.
pub(super) fn find(words: &[Word]) -> Peel<'_> {
    // The words after each action keyword, up to the next one; the command ends at `;` or `+`.
    let actions: Vec<&[Word]> = words
        .split(|word| FIND_EXEC.contains(&word.text.as_str()))
        .skip(1)
        .filter_map(|after| {
            after
                .split(|word| word.text == ";" || word.text == "+")
                .next()
        })
        .collect();
    if actions.is_empty() {
        Peel::Done(Run::Nothing)
    } else {
        Peel::Done(Run::Exec(actions))
    }
}
