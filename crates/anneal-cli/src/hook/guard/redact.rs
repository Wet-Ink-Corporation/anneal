//! The Bash command as Clef D1 and the decision log see it: a skeleton of programs, flags and
//! separators, where every argument value is [`REDACTED`] unless an allowlist ([`allow`])
//! names it. It is built from the guard's own lexer ([`shell::parse`]) and the denylist's
//! wrapper table ([`denylist::layout`]), the view the denylist judges, so redaction never reads
//! bash differently from the guard: no argument text reaches Clef unless a fixed table names it.
//!
//! Each simple command the lexer lists (those of `$( )`, backticks, subshells and process
//! substitutions come out as their own, before the command holding them) is rendered in order:
//!
//! - **Words before the program**: keywords (`if`, `{`, `!`) as written; assignments as
//!   `NAME=<redacted>` (a name over 64 characters is hidden too); the denylist's wrappers
//!   (`sudo`, `env`, `nohup`, `timeout`, `nice`, `exec`, `command`, `xargs`, ...) by the program
//!   rule, their options by the option rule and their values (`sudo -u root`, `timeout 5`)
//!   hidden, so the program after any wrapper is shown as a bare one is.
//! - **The program word**: kept when it is literal and 1 to 64 of `[A-Za-z0-9._+/-]`
//!   ([`allow::is_program`]); otherwise hidden.
//! - **Options**: `--` and `-` kept; a long option's name kept when it is 1 to 64 of
//!   `[A-Za-z0-9._-]`, its `=value` hidden (`--token=<redacted>`); a short option kept when it
//!   is one ASCII letter or digit, or a cluster of the program's flags that take no value
//!   (`rm -rf`, `git clean -fdx`), or one of the program's exact single-dash flags
//!   (`terraform -auto-approve`, `find -delete`); any other is `-<redacted>`, since its letters
//!   may hold a value (`mysql -psecret`).
//! - **Arguments**: hidden, except a subcommand of the program (`git push`, `kubectl delete`,
//!   `docker system prune`), the key of a keyed operand (`dd of=`), and a sensitive target
//!   (`/`, `~`, `.`, `*`, `HEAD`, `main`, `origin`, `/dev/null`, `/dev/sda`, ...).
//! - **Redirections** at the end: `> <target>` for each file written (any write operator is
//!   shown as `>`), `< <redacted>` for a file read, `<<< <redacted>` for a heredoc or
//!   here-string; the target hidden unless it is a sensitive target. Comments are dropped.
//! - **Bodies**: a `bash -c`, `eval`, `su -c`, `trap` or `watch` body (the denylist's
//!   [`denylist::Layout::Script`]) that is literal is lexed and rendered in turn, in `'…'`, at
//!   most [`MAX_NESTING`] deep and within the denylist's lexing budget; any other body is
//!   hidden. `find`'s actions are rendered as commands.
//!
//! Commands are joined by `; `, or ` | ` when a command reads the pipe of the one just before
//! it: the lexer keeps no other separator. A command the lexer cannot read is [`UNPARSED`],
//! nothing else. The skeleton is trimmed and cut to [`MAX_COMMAND_CHARS`].

mod allow;

use std::borrow::Cow;
use std::ops::Range;
use std::ptr;

use super::denylist::{self, Layout};
use super::shell::{self, MAX_NESTING, ParseError, Shape, Stdin, Word};
use crate::decide::redact::REDACTED;
use crate::decide::state::cut_chars;

/// Most characters of the command kept, after redaction; a longer one ends in `…`. The same as
/// `decide::state::MAX_STRING_CHARS`, so the compact state does not cut it again.
pub(crate) const MAX_COMMAND_CHARS: usize = crate::decide::state::MAX_STRING_CHARS;

/// What a command the guard's lexer cannot read becomes, whole.
pub(crate) const UNPARSED: &str = "<unparsed>";

/// Most bytes rendered for one text before the rest is dropped (H-13): enough for the cut of
/// [`MAX_COMMAND_CHARS`] characters, each at most 4 bytes.
const MAX_RENDERED_BYTES: usize = MAX_COMMAND_CHARS * 4 + 64;

/// Most bytes lexed for one command, nested bodies included: the denylist's budget.
const MAX_LEXED_BYTES: usize = denylist::MAX_LEXED_MIB * 1024 * 1024;

/// `command` as its skeleton, trimmed and cut to [`MAX_COMMAND_CHARS`]; [`UNPARSED`] when the
/// guard's lexer cannot read it.
pub(crate) fn redact(command: &str) -> String {
    let mut skeleton = Skeleton {
        lexed: command.len(),
        depth: 0,
    };
    let rendered = or_unparsed(skeleton.render(command));
    cut_chars(rendered.trim(), MAX_COMMAND_CHARS)
}

/// The skeleton rendered, or [`UNPARSED`] for a text the lexer refused, whatever the reason.
fn or_unparsed(rendered: Result<String, ParseError>) -> String {
    match rendered {
        Ok(rendered) => rendered,
        Err(
            ParseError::Unclosed { .. } | ParseError::TooDeep | ParseError::MissingRedirectTarget,
        ) => UNPARSED.to_owned(),
    }
}

/// One rendering: the bytes lexed so far, nested bodies included, and how deep the text being
/// rendered is nested in bodies and `find` actions.
#[derive(Debug)]
struct Skeleton {
    lexed: usize,
    depth: usize,
}

/// The rendered words of one text, separated by single spaces, bounded by
/// [`MAX_RENDERED_BYTES`].
#[derive(Debug, Default)]
struct Line {
    text: String,
}

impl Line {
    fn is_full(&self) -> bool {
        self.text.len() > MAX_RENDERED_BYTES
    }

    /// Appends `token`, after a space unless it starts the line or follows a separator.
    fn word(&mut self, token: &str) {
        if self.is_full() {
            return;
        }
        if !self.text.is_empty() && !self.text.ends_with(' ') {
            self.text.push(' ');
        }
        self.text.push_str(token);
    }

    /// Appends a separator between commands.
    fn separator(&mut self, separator: &str) {
        if !self.is_full() {
            self.text.push_str(separator);
        }
    }
}

impl Skeleton {
    /// The skeleton of `text`, at the current depth.
    fn render(&mut self, text: &str) -> Result<String, ParseError> {
        let commands = shell::parse(text)?;
        let mut line = Line::default();
        for (index, command) in commands.iter().enumerate() {
            if line.is_full() {
                break;
            }
            if index > 0 {
                let piped = command.stdin == Stdin::Pipe(index.checked_sub(1));
                line.separator(if piped { " | " } else { "; " });
            }
            self.words(&command.words, &mut line);
            for target in &command.writes {
                line.word(">");
                line.word(&shown_target(target));
            }
            match command.stdin {
                Stdin::File => line.word("< <redacted>"),
                Stdin::Text(_) => line.word("<<< <redacted>"),
                Stdin::Inherited | Stdin::Pipe(_) => {}
            }
        }
        Ok(line.text)
    }

    /// Runs `render` one level deeper; `None` past [`MAX_NESTING`].
    fn deeper<T>(&mut self, render: impl FnOnce(&mut Self) -> T) -> Option<T> {
        let depth = self.depth;
        if depth >= MAX_NESTING {
            return None;
        }
        self.depth = depth.saturating_add(1);
        let rendered = render(self);
        self.depth = depth;
        Some(rendered)
    }

    /// The skeleton of a body a wrapper runs, one level deeper: [`REDACTED`] past
    /// [`MAX_NESTING`] or the lexing budget, [`UNPARSED`] when it cannot be read.
    fn nested(&mut self, text: &str) -> String {
        self.lexed = self.lexed.saturating_add(text.len());
        if self.lexed > MAX_LEXED_BYTES {
            return REDACTED.to_owned();
        }
        self.deeper(|skeleton| or_unparsed(skeleton.render(text)))
            .unwrap_or_else(|| REDACTED.to_owned())
    }

    /// Renders one simple command's words, as the denylist lays them out.
    fn words(&mut self, words: &[Word], line: &mut Line) {
        let assigned = denylist::assignments(words);
        let all = Lead {
            words,
            assigned: &assigned,
        };
        match denylist::layout(words) {
            Layout::Program(at) => {
                let (lead, rest) = all.split_at(at);
                render_lead(lead, None, line);
                if let Some((program, args)) = rest.words.split_first() {
                    line.word(&shown_program(program));
                    let name = denylist::program_name(program);
                    for arg in args {
                        line.word(&shown_argument(arg, name));
                    }
                }
            }
            Layout::Script(body) => self.script(all, &body, line),
            Layout::Exec(actions) => self.exec(all, &actions, line),
            Layout::Other => render_lead(all, None, line),
        }
    }

    /// A command whose wrappers run the shell text `body`: the words it came from become its
    /// skeleton in `'…'` (or [`REDACTED`] when they are not literal), the others are rendered
    /// as wrapper words.
    fn script(&mut self, all: Lead<'_>, body: &str, line: &mut Line) {
        let Some(source) = body_source(all.words, body) else {
            // A body inside a word or built from several (`env -Sbody`, `alias`): the words
            // are rendered as they are, then the body after them when every word is literal.
            render_lead(all, None, line);
            if all.words.iter().all(Word::is_literal) {
                line.word(&quoted(&self.nested(body)));
            }
            return;
        };
        let (before, rest) = all.split_at(source.start);
        let (source, after) = rest.split_at(source.len());
        render_lead(before, None, line);
        if source.words.iter().all(Word::is_literal) {
            line.word(&quoted(&self.nested(body)));
        } else {
            line.word(REDACTED);
        }
        render_lead(after, None, line);
    }

    /// `find` with actions: its own words as wrapper words, each action as a command one
    /// level deeper.
    fn exec(&mut self, all: Lead<'_>, actions: &[Range<usize>], line: &mut Line) {
        let find = all.words.first().and_then(denylist::program_name);
        // `rest` is `all` from index `at`; the actions come in order and do not overlap.
        let (mut rest, mut at) = (all, 0);
        for range in actions {
            let (before, tail) = rest.split_at(range.start.saturating_sub(at));
            let (action, tail) = tail.split_at(range.len());
            render_lead(before, find, line);
            let rendered = self.deeper(|skeleton| skeleton.words(action.words, line));
            if rendered.is_none() {
                line.word(REDACTED);
            }
            (rest, at) = (tail, range.end);
        }
        render_lead(rest, find, line);
    }
}

/// Words outside a program's arguments, and which of them the denylist reads as assignments
/// (`denylist::assignments`), index for index.
#[derive(Debug, Clone, Copy)]
struct Lead<'w> {
    words: &'w [Word],
    assigned: &'w [bool],
}

impl Lead<'_> {
    /// The words before index `at` and from it; past the end, all of them and none.
    fn split_at(self, at: usize) -> (Self, Self) {
        let (words, words_after) = self.words.split_at_checked(at).unwrap_or((self.words, &[]));
        let (assigned, assigned_after) = self
            .assigned
            .split_at_checked(at)
            .unwrap_or((self.assigned, &[]));
        (
            Self { words, assigned },
            Self {
                words: words_after,
                assigned: assigned_after,
            },
        )
    }
}

/// Where in `words` the word or words a body came from are: the word whose text it is, or the
/// last words that join to it with spaces (`eval`, `watch`).
fn body_source(words: &[Word], body: &str) -> Option<Range<usize>> {
    // A borrowed body is a word's own text when it has that word's address and length.
    match words
        .iter()
        .position(|word| ptr::eq(word.text.as_str(), body))
    {
        Some(at) => Some(at..at.saturating_add(1)),
        None => Some(joined_suffix(words, body)?..words.len()),
    }
}

/// Where the last words of `words` start whose texts joined by spaces are `body`.
fn joined_suffix(words: &[Word], body: &str) -> Option<usize> {
    let mut length = 0_usize;
    for (at, word) in words.iter().enumerate().rev() {
        if length > 0 {
            length = length.saturating_add(1);
        }
        length = length.saturating_add(word.text.len());
        if length == body.len() {
            let suffix = words.get(at..)?;
            let joined = suffix
                .iter()
                .map(|word| word.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            return (joined == body).then_some(at);
        }
        if length > body.len() {
            return None;
        }
    }
    None
}

/// A nested skeleton as one word.
fn quoted(skeleton: &str) -> String {
    format!("'{skeleton}'")
}

/// Renders words outside a program's arguments: keywords, assignments, wrappers with their
/// options and values. `program` names the wrapper before them, for its option tables. A
/// sensitive target is only that: `.` also names `source`, but `find .` stays `find`'s.
fn render_lead(lead: Lead<'_>, program: Option<&str>, line: &mut Line) {
    let mut wrapper = program;
    for (at, word) in lead.words.iter().enumerate() {
        if denylist::is_keyword(word) {
            line.word(&word.text);
        } else if lead.assigned.get(at).copied().unwrap_or(false) {
            line.word(&shown_assignment(word));
        } else if word.shape != Shape::Dynamic && allow::is_target(&word.text) {
            line.word(&word.text);
        } else if denylist::names_wrapper(word) {
            wrapper = denylist::program_name(word);
            line.word(&shown_program(word));
        } else {
            line.word(&shown_argument(word, wrapper));
        }
    }
}

/// A program word: as written when literal and of [`allow::is_program`]'s shape.
fn shown_program(word: &Word) -> Cow<'_, str> {
    if word.is_literal() && allow::is_program(&word.text) {
        Cow::Borrowed(&word.text)
    } else {
        Cow::Borrowed(REDACTED)
    }
}

/// An assignment before the program: its name, if short, and the value hidden.
fn shown_assignment(word: &Word) -> Cow<'static, str> {
    match word.text.split_once('=') {
        Some((name, _)) if name.len() <= allow::MAX_NAME_CHARS => {
            Cow::Owned(format!("{name}={REDACTED}"))
        }
        _ => Cow::Owned(format!("{REDACTED}={REDACTED}")),
    }
}

/// An argument of `program`: an option by the option rule, an allowlisted word as written,
/// anything else hidden.
fn shown_argument<'w>(word: &'w Word, program: Option<&str>) -> Cow<'w, str> {
    let text = word.text.as_str();
    if word.shape == Shape::Dynamic {
        return Cow::Borrowed(REDACTED);
    }
    if word.is_literal() && text.starts_with('-') {
        return shown_option(text, program);
    }
    if allow::is_target(text) || (word.is_literal() && allow::is_subcommand(program, text)) {
        return Cow::Borrowed(text);
    }
    match text.split_once('=') {
        Some((key, value)) if word.is_literal() && allow::is_key(program, key) => {
            let value = if allow::is_target(value) {
                value
            } else {
                REDACTED
            };
            Cow::Owned(format!("{key}={value}"))
        }
        _ => Cow::Borrowed(REDACTED),
    }
}

/// A literal word starting with `-`, by the option rule of the module documentation.
fn shown_option<'w>(text: &'w str, program: Option<&str>) -> Cow<'w, str> {
    if text == "-" || text == "--" || allow::is_exact_flag(program, text) {
        return Cow::Borrowed(text);
    }
    if let Some(long) = text.strip_prefix("--") {
        return match long.split_once('=') {
            Some((name, _)) if allow::is_option_name(name) => {
                Cow::Owned(format!("--{name}={REDACTED}"))
            }
            None if allow::is_option_name(long) => Cow::Borrowed(text),
            _ => Cow::Borrowed(REDACTED),
        };
    }
    match text.strip_prefix('-') {
        Some(letters) if allow::is_flag_cluster(program, letters) => Cow::Borrowed(text),
        _ => Cow::Owned(format!("-{REDACTED}")),
    }
}

/// A redirection's target: as written when it is a sensitive target, else hidden.
fn shown_target(word: &Word) -> Cow<'_, str> {
    if word.shape != Shape::Dynamic && allow::is_target(&word.text) {
        Cow::Borrowed(&word.text)
    } else {
        Cow::Borrowed(REDACTED)
    }
}

#[cfg(test)]
mod tests;
