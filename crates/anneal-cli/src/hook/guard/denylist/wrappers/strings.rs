//! Wrappers that run a command string or read a script: shells, `sudo -s`, `env -S`, `su`,
//! `script`, `trap`, `watch`, `flock -c`, and `find`'s actions.

use std::borrow::Cow;

use super::{
    Long, Peel, Run, SUDO_VALUES, is_long, is_option, join, skip_one, skip_options, takes_value,
};
use crate::hook::guard::shell::Word;

const ENV_VALUES: &[&str] = &["-u", "-C", "--unset", "--chdir"];
const SHELL_VALUES: &[&str] = &["--rcfile", "--init-file"];
const FLOCK_VALUES: &[&str] = &["-w", "-E", "--timeout", "--conflict-exit-code"];
const WATCH_VALUES: &[&str] = &["-n", "--interval", "-q", "--equexit"];
const SU_VALUES: &[&str] = &["-s", "-g", "-G", "-w", "--shell", "--group", "--supp-group"];

/// The `find` actions that run a command on each match.
const FIND_EXEC: [&str; 4] = ["-exec", "-execdir", "-ok", "-okdir"];

/// `sudo` and `doas`: the command after their options, or, with `-s` or `-i` and no command,
/// a shell reading stdin.
pub(super) fn sudo(words: &[Word]) -> Peel<'_> {
    let rest = skip_options(words, SUDO_VALUES);
    let mut options = words.iter().take(words.len().saturating_sub(rest.len()));
    let shell = options.any(|word| {
        is_long(&word.text, "--shell")
            || is_long(&word.text, "--login")
            || (is_option(word) && !word.text.starts_with("--") && word.text.contains(['s', 'i']))
    });
    if rest.is_empty() && shell {
        Peel::Done(Run::StdinScript("sudo"))
    } else {
        Peel::Inner(rest)
    }
}

/// `env`'s command after its options, or the script `-S` (`--split-string`) runs.
pub(super) fn env(words: &[Word]) -> Peel<'_> {
    let mut rest = words;
    while let [first, tail @ ..] = rest {
        let text = first.text.as_str();
        let split = if text == "-S" {
            Some((tail.first().map(|w| w.text.as_str()), skip_one(tail)))
        } else {
            long_value(text, tail, "--split-string").or_else(|| {
                text.strip_prefix("-S")
                    .filter(|s| !s.is_empty())
                    .map(|attached| (Some(attached), tail))
            })
        };
        if let Some((script, after)) = split.filter(|_| first.is_literal()) {
            // `-S` as the last word splits nothing: an empty script.
            let script = script.unwrap_or("");
            return Peel::Done(Run::Script(if after.is_empty() {
                Cow::Borrowed(script)
            } else {
                Cow::Owned(format!("{script} {}", join(after)))
            }));
        }
        if text == "--" {
            return Peel::Inner(tail);
        }
        if !is_option(first) {
            break;
        }
        rest = if takes_value(text, ENV_VALUES) {
            skip_one(tail)
        } else {
            tail
        };
    }
    Peel::Inner(rest)
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
    let after_file = skip_one(skip_options(words, FLOCK_VALUES));
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

/// `su [options] [USER]`: the `-c` string, or, without one, a shell reading stdin.
pub(super) fn su(words: &[Word]) -> Peel<'_> {
    let mut rest = words;
    while let [first, tail @ ..] = rest {
        let text = first.text.as_str();
        if let Some((command, _)) = long_value(text, tail, "--command") {
            return command_string(command);
        }
        if text == "-c" {
            return command_string(tail.first().map(|command| command.text.as_str()));
        }
        rest = if takes_value(text, SU_VALUES) {
            skip_one(tail)
        } else {
            tail
        };
    }
    Peel::Done(Run::StdinScript("su"))
}

/// `script [options] [FILE]`: the `-c` string, or, without one, a shell reading stdin.
pub(super) fn script(words: &[Word]) -> Peel<'_> {
    let mut rest = words;
    while let [first, tail @ ..] = rest {
        rest = tail;
        let text = first.text.as_str();
        if let Some((command, _)) = long_value(text, tail, "--command") {
            return command_string(command);
        }
        let cluster = is_option(first) && !text.starts_with("--");
        let command = text
            .split_once('c')
            .filter(|_| cluster)
            .map(|(_, attached)| attached);
        match command {
            // `-c` last in its cluster: the next word is the command.
            Some("") => {
                return command_string(tail.first().map(|command| command.text.as_str()));
            }
            Some(attached) => return Peel::Done(Run::Script(Cow::Borrowed(attached))),
            None => {}
        }
    }
    Peel::Done(Run::StdinScript("script"))
}

/// `watch [options] command...`: the words joined are run by `sh -c`.
pub(super) fn watch(words: &[Word]) -> Peel<'_> {
    let rest = skip_options(words, WATCH_VALUES);
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
        let option = first.is_literal() && (text.starts_with('-') || text.starts_with('+'));
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
    if script.is_literal() && STDIN_PATHS.contains(&path) {
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
