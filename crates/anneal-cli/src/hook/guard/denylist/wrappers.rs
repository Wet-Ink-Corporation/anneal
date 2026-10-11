//! Unwrapping a simple command down to the program that really runs.

use std::borrow::Cow;

use crate::hook::guard::shell::Word;

/// Words that may start a simple command before its program.
const KEYWORDS: [&str; 9] = [
    "!", "{", "if", "then", "else", "elif", "do", "while", "until",
];

/// Shells whose `-c` string is a command.
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
const ENV_VALUES: &[&str] = &["-u", "-C", "--unset", "--chdir"];
const EXEC_VALUES: &[&str] = &["-a"];
const TIME_VALUES: &[&str] = &["-f", "-o", "--format", "--output"];
const NICE_VALUES: &[&str] = &["-n", "--adjustment"];
const TIMEOUT_VALUES: &[&str] = &["-s", "-k", "--signal", "--kill-after"];
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
const SHELL_VALUES: &[&str] = &["--rcfile", "--init-file"];

/// The `find` actions that run a command on each match.
const FIND_EXEC: [&str; 4] = ["-exec", "-execdir", "-ok", "-okdir"];

/// The program of a simple command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Program<'a> {
    /// Its name, without a path or `.exe`.
    Named(&'a str),
    /// A name only known at run time (`$(echo rm)`, `"$GIT"`, a glob).
    Unknown,
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
    /// Shell text to read in turn: a `-c` string, `eval`'s arguments, `env -S`.
    Script(Cow<'a, str>),
    /// The words `find -exec` runs, with each match appended.
    Exec(&'a [Word]),
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
        let Program::Named(name) = program else {
            return Run::Command {
                program,
                args: rest,
                appended,
            };
        };
        words = match name {
            "sudo" | "doas" => skip_prefix(skip_options(rest, SUDO_VALUES)),
            "env" => match env(rest) {
                Ok(rest) => skip_prefix(rest),
                Err(script) => return Run::Script(script),
            },
            "command" | "nohup" | "builtin" => skip_options(rest, &[]),
            "exec" => skip_options(rest, EXEC_VALUES),
            "time" => skip_options(rest, TIME_VALUES),
            "nice" => skip_options(rest, NICE_VALUES),
            "timeout" => skip_one(skip_options(rest, TIMEOUT_VALUES)),
            "xargs" => {
                appended = true;
                skip_options(rest, XARGS_VALUES)
            }
            "eval" => return Run::Script(Cow::Owned(join(rest))),
            "find" => return find_exec(rest).map_or(Run::Nothing, Run::Exec),
            shell if SHELLS.contains(&shell) => {
                return shell_script(rest).map_or(Run::Nothing, |s| Run::Script(Cow::Borrowed(s)));
            }
            _ => {
                return Run::Command {
                    program,
                    args: rest,
                    appended,
                };
            }
        };
    }
}

/// The program `word` names: its last path component without `.exe`, or unknown when the
/// word is not literal.
fn program(word: &Word) -> Program<'_> {
    if !word.is_literal() {
        return Program::Unknown;
    }
    let text = word.text.as_str();
    let name = text.rsplit(['/', '\\']).next().unwrap_or(text);
    Program::Named(name.strip_suffix(".exe").unwrap_or(name))
}

/// Skips leading keywords (`if`, `then`, `{`, `!`, ...) and `NAME=value` assignments.
fn skip_prefix(words: &[Word]) -> &[Word] {
    let mut rest = words;
    while let [first, tail @ ..] = rest {
        let keyword = !first.quoted && KEYWORDS.contains(&first.text.as_str());
        if !(keyword || is_assignment(first)) {
            break;
        }
        rest = tail;
    }
    rest
}

/// `NAME=value`, with `NAME` a shell identifier.
fn is_assignment(word: &Word) -> bool {
    word.text.split_once('=').is_some_and(|(name, _)| {
        name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Whether `word` is an option: literal, starting with `-`, and not `-` alone.
pub(super) fn is_option(word: &Word) -> bool {
    word.is_literal() && word.text.starts_with('-') && word.text != "-"
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
fn join(words: &[Word]) -> String {
    words
        .iter()
        .map(|word| word.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `env`'s operands after its options, or the script `-S` (`--split-string`) runs.
fn env(words: &[Word]) -> Result<&[Word], Cow<'_, str>> {
    let mut rest = words;
    while let [first, tail @ ..] = rest {
        let text = first.text.as_str();
        let split = match text {
            "-S" | "--split-string" => {
                Some((tail.first().map(|w| w.text.as_str()), skip_one(tail)))
            }
            _ => text
                .strip_prefix("--split-string=")
                .or_else(|| text.strip_prefix("-S").filter(|s| !s.is_empty()))
                .map(|attached| (Some(attached), tail)),
        };
        if let Some((script, after)) = split.filter(|_| first.is_literal()) {
            // `-S` as the last word splits nothing: an empty script.
            let script = script.unwrap_or("");
            return Err(if after.is_empty() {
                Cow::Borrowed(script)
            } else {
                Cow::Owned(format!("{script} {}", join(after)))
            });
        }
        if text == "--" {
            return Ok(tail);
        }
        if !is_option(first) {
            break;
        }
        rest = if ENV_VALUES.contains(&text) {
            skip_one(tail)
        } else {
            tail
        };
    }
    Ok(rest)
}

/// The string a shell's `-c` runs: the first operand after options that include `c`.
fn shell_script(words: &[Word]) -> Option<&str> {
    let mut rest = words;
    let mut command_string = false;
    while let [first, tail @ ..] = rest {
        let text = first.text.as_str();
        if text == "--" || text == "-" {
            rest = tail;
            break;
        }
        let short = first.is_literal() && (text.starts_with('-') || text.starts_with('+'));
        if !short {
            break;
        }
        let long = text.starts_with("--");
        command_string |= !long && text.starts_with('-') && text.contains('c');
        let takes_value = if long {
            SHELL_VALUES.contains(&text)
        } else {
            text.ends_with(['o', 'O'])
        };
        rest = if takes_value { skip_one(tail) } else { tail };
    }
    if command_string {
        rest.first().map(|word| word.text.as_str())
    } else {
        None
    }
}

/// The command `find -exec` (or `-execdir`, `-ok`, `-okdir`) runs, up to its `;` or `+`.
fn find_exec(words: &[Word]) -> Option<&[Word]> {
    let start = words
        .iter()
        .position(|word| FIND_EXEC.contains(&word.text.as_str()))?;
    let command = words.get(start.checked_add(1)?..)?;
    let end = command
        .iter()
        .position(|word| word.text == ";" || word.text == "+")
        .unwrap_or(command.len());
    command.get(..end)
}
