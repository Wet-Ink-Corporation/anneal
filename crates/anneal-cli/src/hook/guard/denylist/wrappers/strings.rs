//! Wrappers that run a command string or read a script: shells, `sudo -s`, `env -S`, `su`,
//! `script`, `trap`, `watch`, `flock -c`, and `find`'s actions.

use std::borrow::Cow;
use std::collections::HashSet;

use super::grammar::{Grammar, Name, Parsed, Read};
use super::tables::{ENV, FLOCK, SCRIPT, SU, SU_TO_USER, SUDO, WATCH};
use super::{Long, Peel, Run, is_option, join, skip_one, skip_options};
use crate::hook::guard::denylist::braces::reads_as_written;
use crate::hook::guard::shell::Word;

/// What one of bash's long options does.
#[derive(Clone, Copy)]
enum ShellLong {
    /// A flag.
    Flag,
    /// It takes the next word.
    Value,
    /// bash prints and runs nothing.
    Exits,
}

/// bash's long options (bash 5.2.21 as built by Debian and Ubuntu), each written with one dash
/// or two and only before its first short option.
const SHELL_LONG: [(&str, ShellLong); 16] = [
    ("debug", ShellLong::Flag),
    ("debugger", ShellLong::Flag),
    ("dump-po-strings", ShellLong::Flag),
    ("dump-strings", ShellLong::Flag),
    ("help", ShellLong::Exits),
    ("init-file", ShellLong::Value),
    ("login", ShellLong::Flag),
    ("noediting", ShellLong::Flag),
    ("noprofile", ShellLong::Flag),
    ("norc", ShellLong::Flag),
    ("posix", ShellLong::Flag),
    ("pretty-print", ShellLong::Flag),
    ("rcfile", ShellLong::Value),
    ("restricted", ShellLong::Flag),
    ("verbose", ShellLong::Flag),
    ("version", ShellLong::Exits),
];

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
        // The shell read the words after the string, groups included: that reading joins them
        // to it as text, as before. env splits its string itself, with its own escapes and no
        // group read, and keeps each word after it whole: those readings quote them, with the
        // string as env reads it and, in case env reads it otherwise, as written. Either
        // denial denies.
        let joined = if split.after.is_empty() {
            Cow::Borrowed(script)
        } else {
            Cow::Owned(format!("{script} {}", join(split.after)))
        };
        return Peel::Done(Run::either([
            Run::Script(joined),
            Run::Split(followed_by(as_env_args(&split_by_env(script)), split.after)),
            Run::Split(followed_by(as_env_args(script), split.after)),
        ]));
    }
    match read.rest {
        [dash, tail @ ..] if dash.text == "-" && reads_as_written(dash) => Peel::Inner(tail),
        rest => Peel::Inner(rest),
    }
}

/// The characters `env -S` splits words at, beside `\_`.
const ENV_BLANKS: [char; 6] = [' ', '\t', '\n', '\r', '\u{b}', '\u{c}'];

/// `script` as shell text that splits as `env -S` splits it (GNU coreutils 9.4):
/// - outside quotes, a LF (which the shell reads as a separator of commands), CR, VT or FF
///   separates words, as a blank does, and `;`, `&`, `|`, `<`, `>`, `(` and `)` are characters
///   of their word, since env reads no operator;
/// - outside single quotes a backtick is a character of its word, since env runs no
///   substitution (a `$` other than `${NAME}` makes env refuse the string);
/// - outside single quotes, `\_` is a blank (a separator outside quotes, a space inside double
///   quotes), `\t`, `\n`, `\r`, `\f` and `\v` are those characters kept in their word, and `\c`
///   ends the string, as an unquoted `#` starting a word does;
/// - inside double quotes `\'` and `\#` are the character;
/// - inside single quotes `\'` is a quote and `\\` a backslash, the quote staying open.
///
/// The other escapes (`\\`, `\"`, `\$`, `\#` unquoted) read as the shell reads them.
fn split_by_env(script: &str) -> Cow<'_, str> {
    const SPECIAL: [char; 14] = [
        '\\', '#', '\n', '\r', '\u{b}', '\u{c}', ';', '&', '|', '<', '>', '(', ')', '`',
    ];
    if !script.contains(SPECIAL) {
        return Cow::Borrowed(script);
    }
    let mut text = String::with_capacity(script.len());
    let mut quote = None;
    let mut chars = script.chars().peekable();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some('\''), '\\') => match chars.next_if(|next| matches!(next, '\\' | '\'')) {
                Some('\'') => text.push_str(r"'\''"),
                // `\\` is one backslash, and a lone one is itself.
                _ => text.push(c),
            },
            (_, '\\') => match (chars.next(), quote) {
                (Some('_'), _) => text.push(' '),
                // Inside double quotes env refuses `\c` and runs nothing; the quote left open
                // there makes the reading unparseable, which only denies.
                (Some('c'), _) => break,
                (Some(next @ ('t' | 'n' | 'r' | 'f' | 'v')), _) => {
                    let control = match next {
                        't' => '\t',
                        'n' => '\n',
                        'r' => '\r',
                        'f' => '\u{c}',
                        _ => '\u{b}',
                    };
                    if quote.is_some() {
                        text.push(control);
                    } else {
                        push_quoted(&mut text, control);
                    }
                }
                (Some(next @ ('\'' | '#')), Some(_)) => text.push(next),
                (Some(next), _) => {
                    text.push(c);
                    text.push(next);
                }
                (None, _) => text.push(c),
            },
            (None, '\n' | '\r' | '\u{b}' | '\u{c}') => text.push(' '),
            (None, '#') if text.is_empty() || text.ends_with(ENV_BLANKS) => break,
            (None, ';' | '&' | '|' | '<' | '>' | '(' | ')' | '`') => push_quoted(&mut text, c),
            (Some('"'), '`') => {
                text.push('"');
                push_quoted(&mut text, c);
                text.push('"');
            }
            (None, '\'' | '"') => {
                quote = Some(c);
                text.push(c);
            }
            (Some(open), _) if c == open => {
                quote = None;
                text.push(c);
            }
            _ => text.push(c),
        }
    }
    Cow::Owned(text)
}

/// `split` run by `env` again: env reads the words it split from its string as its own
/// arguments, options (a nested `-S` among them) and assignments first.
fn as_env_args(split: &str) -> Cow<'static, str> {
    Cow::Owned(format!("env {split}"))
}

/// Pushes `c` single-quoted, so the shell reads it as a character of its word.
fn push_quoted(text: &mut String, c: char) {
    text.push('\'');
    text.push(c);
    text.push('\'');
}

/// `script` followed by each word of `after`, single-quoted whole.
fn followed_by<'a>(script: Cow<'a, str>, after: &[Word]) -> Cow<'a, str> {
    if after.is_empty() {
        return script;
    }
    Cow::Owned(after.iter().fold(script.into_owned(), |mut text, word| {
        text.push_str(" '");
        text.push_str(&word.text.replace('\'', r"'\''"));
        text.push('\'');
        text
    }))
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
/// (`--login`), or a `-` among its operands or the words after `--`, where su may read its
/// `-` too. `su` takes options after the user too.
fn su_login(words: &[Word]) -> bool {
    let read = SU.parse(words);
    read.holds('l', "login")
        || read
            .operands
            .iter()
            .copied()
            .chain(read.rest)
            .any(login_dash)
}

/// Whether `option` is a `-c` (`--command`, and for `su` `--session-command`).
fn is_command(option: &Parsed<'_>) -> bool {
    matches!(
        option.name,
        Name::Short('c') | Name::Long("command" | "session-command")
    )
}

/// The command strings of the `-c` options among `options`, in order; `None` for one missing
/// its string.
fn commands(options: Vec<Parsed<'_>>) -> impl Iterator<Item = Option<&str>> {
    options
        .into_iter()
        .filter(is_command)
        .map(|option| option.value)
}

/// What the `-c` strings among `options` run: the last one runs, the earlier ones are read
/// too, each on its own (never joined, where one could swallow the next) and each distinct
/// string once. `None` without a `-c`; with only a `-c` missing its string, nothing.
fn bodies(options: Vec<Parsed<'_>>) -> Option<Run<'_>> {
    let mut commands = commands(options).peekable();
    commands.peek()?;
    let mut seen = HashSet::new();
    let mut runs: Vec<Run<'_>> = commands
        .flatten()
        .filter(|body| seen.insert(*body))
        .map(|body| Run::Script(Cow::Borrowed(body)))
        .collect();
    Some(if runs.len() > 1 {
        Run::Either(runs)
    } else {
        runs.pop().unwrap_or(Run::Nothing)
    })
}

/// Whether `word` is su's lone `-` (`--login`'s old spelling) before its user.
fn login_dash(word: &Word) -> bool {
    word.text == "-" && reads_as_written(word)
}

/// `su [options] [USER] [ARG...]`: the user's shell runs `-c` and su's last `-c` string, if
/// any, then su's arguments after the user, read as bash reads them. getopt permutes su's
/// options and operands; su reads its `-` and user from its operands followed by the words
/// after `--` and hands the shell the rest (which may be `-` or bash's `+x`, `+o NAME`). With
/// `POSIXLY_CORRECT` set getopt stops at the user and su hands the shell every word after it.
/// Each reading is checked, every `-c` string, and, without a `-c`, the words after `--` on
/// their own.
pub(super) fn su(words: &[Word]) -> Peel<'_> {
    let Read {
        options,
        rest: after_dashes,
        operands,
    } = SU.parse(words);
    let to_user = SU_TO_USER.parse(words);
    // su takes a lone `-` first, then the user. Under POSIXLY_CORRECT a `--` after that `-`
    // is the user, and su finds no such user; it is read as ending su's options instead.
    let user = match to_user.rest {
        [dash, ends, tail @ ..] if login_dash(dash) && ends_options(ends) => tail,
        [dash, tail @ ..] if login_dash(dash) => tail,
        rest => rest,
    };
    let posix = su_reading(to_user.options, skip_one(user));
    // getopt hands su its permuted operands, then the words after `--`: the user may be
    // either.
    let positional: Vec<&Word> = operands.into_iter().chain(after_dashes).collect();
    let user = match positional.as_slice() {
        [dash, rest @ ..] if login_dash(dash) => rest,
        rest => rest,
    };
    let argv = user.split_first().map_or(user, |(_, argv)| argv);
    let alone = (!options.iter().any(is_command)).then(|| {
        // The words after `--` that are not su's own `-` or user.
        let from = argv.len().saturating_sub(after_dashes.len());
        let tail = argv.split_at_checked(from).map_or(argv, |(_, tail)| tail);
        shell_run("su", None, tail.iter().copied())
    });
    let permuted = su_reading(options, argv.iter().copied());
    Peel::Done(Run::either([permuted, posix].into_iter().chain(alone)))
}

/// Whether `word` is a literal `--`.
pub(super) fn ends_options(word: &Word) -> bool {
    word.text == "--" && reads_as_written(word)
}

/// One reading of `su`: its options, and `argv`, the words it hands the user's shell after its
/// own `-c` and command string. The `-c` strings run; a last one that bash reads as an option
/// (`su -c -x root 'cmd'`) leaves the command string to `argv`.
fn su_reading<'a, I>(options: Vec<Parsed<'a>>, argv: I) -> Run<'a>
where
    I: IntoIterator<Item = &'a Word>,
    I::IntoIter: Clone,
{
    let lead = options
        .iter()
        .rfind(|option| is_command(option))
        .and_then(|option| option.value)
        .filter(|value| value.starts_with(['-', '+']));
    match (bodies(options), lead) {
        (None, _) => shell_run("su", None, argv),
        (Some(bodies), None) => bodies,
        (Some(bodies), Some(lead)) => Run::either([bodies, shell_run("su", Some(lead), argv)]),
    }
}

/// `script [options] [FILE]`: its `-c` strings, or, without one, a shell reading stdin.
pub(super) fn script(words: &[Word]) -> Peel<'_> {
    Peel::Done(bodies(SCRIPT.parse(words).options).unwrap_or(Run::StdinScript("script")))
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
/// a script read from stdin. Every shell is read with and without bash's long options
/// ([`shell_run`]): a name says neither which shell runs (`sh` may be dash, `busybox bash` is
/// ash) nor whether it has them.
pub(super) fn shell<'a>(name: &'a str, words: &'a [Word]) -> Peel<'a> {
    Peel::Done(shell_run(name, None, words))
}

/// How many of a shell's argument words `words` it reads as options or their values, the word
/// that ends them (its first operand, `--` or `-`) included, in the longer of [`shell_run`]'s
/// two readings: the words where a pattern may stand for an option (#165).
pub(super) fn shell_option_words(words: &[Word]) -> usize {
    [ShellPhase::Long, ShellPhase::Short]
        .into_iter()
        .map(|phase| {
            let mut options = ShellOptions {
                phase,
                ..ShellOptions::default()
            };
            words
                .iter()
                .position(|word| {
                    !matches!(
                        options.read(&word.text, reads_as_written(word)),
                        ShellWord::Option
                    )
                })
                .map_or(words.len(), |at| at.saturating_add(1))
        })
        .max()
        .unwrap_or(0)
}

/// What one word of a shell's arguments is to the shell ([`ShellOptions::read`]).
enum ShellWord {
    /// An option, or an option's value.
    Option,
    /// `--` or `-`: the options end after it.
    Ends,
    /// The first operand.
    Operand,
    /// `--help` or `--version`: the shell runs nothing.
    Exits,
}

/// A shell's options as bash reads them, word by word.
#[derive(Default)]
struct ShellOptions {
    /// `-c` (or `+c`): the first operand is the command string.
    command_string: bool,
    /// `-s` (or `+s`): the script is stdin.
    from_stdin: bool,
    /// Words still owed as values: one for each `o` or `O` read, one for `--rcfile`.
    owed: usize,
    /// Which options bash still reads.
    phase: ShellPhase,
}

/// Which of a shell's options bash still reads.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum ShellPhase {
    /// Long ones and short ones: no short option was read yet.
    #[default]
    Long,
    /// Short ones only.
    Short,
}

impl ShellOptions {
    /// Reads the argument `text` (`literal` when it reads as written).
    fn read(&mut self, text: &str, literal: bool) -> ShellWord {
        if let Some(fewer) = self.owed.checked_sub(1) {
            self.owed = fewer;
            return ShellWord::Option;
        }
        if !literal {
            return ShellWord::Operand;
        }
        if text == "--" || text == "-" {
            return ShellWord::Ends;
        }
        let long = text.strip_prefix("--").or_else(|| text.strip_prefix('-'));
        if let Some(&(_, option)) = long
            .filter(|_| self.phase == ShellPhase::Long)
            .and_then(|long| SHELL_LONG.iter().find(|(name, _)| *name == long))
        {
            return match option {
                ShellLong::Flag => ShellWord::Option,
                ShellLong::Value => {
                    self.owed = 1;
                    ShellWord::Option
                }
                ShellLong::Exits => ShellWord::Exits,
            };
        }
        let Some(letters) = text.strip_prefix('-').or_else(|| text.strip_prefix('+')) else {
            return ShellWord::Operand;
        };
        self.phase = ShellPhase::Short;
        for letter in letters.chars() {
            match letter {
                'c' => self.command_string = true,
                's' => self.from_stdin = true,
                'o' | 'O' => self.owed = self.owed.saturating_add(1),
                _ => {}
            }
        }
        ShellWord::Option
    }
}

/// What the shell `name` runs, given the arguments `words` ([`shell`]); `lead`, when given, is
/// the value of a `-c` already read (su's `-c VALUE`). Both readings are checked: bash's, with
/// its long options open, and that of a shell without them (dash as `sh`, or bash after su's
/// `-c` or `-f`), where every option word is a cluster.
fn shell_run<'a, I>(name: &'a str, lead: Option<&'a str>, words: I) -> Run<'a>
where
    I: IntoIterator<Item = &'a Word>,
    I::IntoIter: Clone,
{
    let words = words.into_iter();
    // Each reading walks the words from the start; the clone copies only the iterator.
    Run::either([
        shell_read(name, lead, words.clone(), ShellPhase::Long),
        shell_read(name, lead, words, ShellPhase::Short),
    ])
}

/// [`shell_run`]'s reading with bash's options open from `phase`.
fn shell_read<'a>(
    name: &'a str,
    lead: Option<&'a str>,
    mut words: impl Iterator<Item = &'a Word>,
    phase: ShellPhase,
) -> Run<'a> {
    let mut options = ShellOptions {
        phase,
        ..ShellOptions::default()
    };
    let mut ended = false;
    if let Some(lead) = lead {
        options.command_string = true;
        match options.read(lead, true) {
            ShellWord::Option => {}
            ShellWord::Ends => ended = true,
            ShellWord::Operand => return Run::Script(Cow::Borrowed(lead)),
            ShellWord::Exits => return Run::Nothing,
        }
    }
    let mut operand = None;
    if !ended {
        for word in words.by_ref() {
            match options.read(&word.text, reads_as_written(word)) {
                ShellWord::Option => {}
                ShellWord::Ends => break,
                ShellWord::Operand => {
                    operand = Some(word);
                    break;
                }
                ShellWord::Exits => return Run::Nothing,
            }
        }
    }
    let first = operand.or_else(|| words.next());
    if options.command_string {
        return first.map_or(Run::Nothing, |string| {
            Run::Script(Cow::Borrowed(&string.text))
        });
    }
    // With `-s` the operands are arguments and the script is stdin.
    match first.filter(|_| !options.from_stdin) {
        None => Run::StdinScript(name),
        Some(script) => script_file(name, script),
    }
}

/// The paths through which a script file is standard input.
const STDIN_PATHS: [&str; 3] = ["/dev/stdin", "/dev/fd/0", "/proc/self/fd/0"];

/// `shell` (or `source`) running the script file `script`: stdin when it names it, unseen
/// when it is a process substitution, else a file the guard does not read.
fn script_file<'a>(shell: &'a str, script: &Word) -> Run<'a> {
    let path = script.text.as_str();
    let descriptor = path.starts_with("/dev/fd/") || path.starts_with("/proc/");
    if reads_as_written(script) && STDIN_PATHS.contains(&path) {
        Run::StdinScript(shell)
    } else if script.is_process_substitution() || descriptor {
        Run::Unseen(shell)
    } else {
        Run::Nothing
    }
}

/// `source FILE` (or `. FILE`).
pub(super) fn source(words: &[Word]) -> Peel<'_> {
    Peel::Done(
        skip_options(words, &[])
            .first()
            .map_or(Run::Nothing, |script| script_file("source", script)),
    )
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

#[cfg(test)]
mod tests {
    use super::split_by_env;
    use crate::hook::guard::shell;

    /// Each string lexes, once `split_by_env` has read it, into the words GNU coreutils 9.4's
    /// `env -S` split it into (observed with `env -S "printf <%q>\_ <string>"`).
    #[test]
    fn a_string_lexes_into_the_words_env_splits_it_into() {
        let table: &[(&str, &[&str])] = &[
            (r"rm\_-rf\_/", &["rm", "-rf", "/"]),
            (r"sh -c rm\t-rf", &["sh", "-c", "rm\t-rf"]),
            (r"a 'b\'c' d", &["a", "b'c", "d"]),
            (r"a 'b\\c'", &["a", r"b\c"]),
            (r#"a "b\'c\#d""#, &["a", "b'c#d"]),
            ("a #b c", &["a"]),
            (r"a\c b", &["a"]),
            ("a b#c", &["a", "b#c"]),
            (r#"a "b\_c""#, &["a", "b c"]),
            (r"a '\_'", &["a", r"\_"]),
            ("a\rb\u{b}c", &["a", "b", "c"]),
            ("a\nb \"c\nd\"", &["a", "b", "c\nd"]),
            ("a;b | c && d;#e", &["a;b", "|", "c", "&&", "d;#e"]),
            ("a (b) <c >>d", &["a", "(b)", "<c", ">>d"]),
            ("a ` b `c \"d`e\"", &["a", "`", "b", "`c", "d`e"]),
            ("a\u{a0}#b", &["a\u{a0}#b"]),
        ];
        let wrong: Vec<String> = table
            .iter()
            .filter_map(|(string, expected)| {
                let read = split_by_env(string);
                // env runs one command: its words are one simple command's.
                let words: Vec<Vec<String>> = shell::parse_without_extglob(&read)
                    .expect("the string lexes")
                    .into_iter()
                    .map(|command| command.words.into_iter().map(|word| word.text).collect())
                    .collect();
                (words != [expected.to_vec()]).then(|| format!("{string:?} lexed as {words:?}"))
            })
            .collect();
        assert!(wrong.is_empty(), "misread:\n{}", wrong.join("\n"));
    }
}
