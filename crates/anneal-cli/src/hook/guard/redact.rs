//! The Bash command as Clef D1 and the decision log see it: a skeleton of programs, flags and
//! separators, where every argument value is [`REDACTED`] unless an allowlist ([`allow`])
//! names it. It is built from the guard's own lexer ([`shell::parse`]) and the denylist's
//! wrapper table ([`denylist::layout`]), the view the denylist judges, so redaction never reads
//! bash differently from the guard: no argument text reaches Clef unless a fixed table names it.
//!
//! Each simple command the lexer lists (those of `$( )`, backticks, subshells and process
//! substitutions come out as their own, before the command holding them) is rendered in order:
//!
//! - **Arithmetic**: a command the lexer read inside `$(( ))` or `(( ))` is one [`REDACTED`],
//!   and a word it read between an unquoted `$[` and its `]` is [`REDACTED`] whatever its role
//!   (an assignment still shows its name): their operands only look like programs and
//!   arguments. A `$( )` or backtick inside one is a command and rendered as one.
//! - **Words before the program**, each by the role the denylist's walk gives it
//!   ([`denylist::layout`]), never by its text alone: keywords (`if`, `{`, `!`) as written;
//!   assignments as `NAME=<redacted>` (a name over 64 characters is hidden too); the words
//!   standing in the place of a program that the walk reads as wrappers (`sudo`, `env`,
//!   `nohup`, `timeout`, `nice`, `exec`, `command`, `xargs`, a shell, `source`, `find`, ...) by
//!   the program rule, their options by the option rule and their values and operands
//!   (`sudo -u root`, `timeout 5`, `bash script`) as arguments, so the program after any
//!   wrapper is shown as a bare one is, and a value that names a wrapper (`sudo -D /x/nice`,
//!   `find /a/env`) is hidden like any other.
//! - **Uncertain wrappers**: the walk is the denylist's, unchanged by redaction, and reads
//!   some wrapper options more loosely than the wrapper does, so a wrapper's words are trusted
//!   only when they read with certainty by [`certain`]'s tables (exact flags, a shell's
//!   cluster of flags ending in `c`, exact value options and their values, `--`, fixed
//!   operands, each value and operand of its slot's shape; `find`'s words before its
//!   actions). From the first word that does not (a cluster such as `-iu`, an option the
//!   tables do not know, a value or operand without its slot's shape such as
//!   `timeout x psql` or one the wrapper may refuse (`xargs -n 0`, `env -u a=b`, any `chrt`
//!   priority), taskset with `-p`, xargs with `-I`, an assignment after a wrapper that takes
//!   none, a non-literal one after a wrapper that takes some (`env K=$V x`, which bash
//!   field-splits), a body followed by a word its wrapper does not let follow it (an option
//!   word or one that may expand, `su -c 'x y' -c ls`, where the last `-c` wins; anything
//!   after `flock -c`'s body; a second file for `script`), a shell option the shell may
//!   refuse or that changes what the body runs (`dash -B`, `bash -l --norc`,
//!   `bash -o nosuch`, `bash -n`, `bash -r`, `bash -k`), `su -s`, an alias name bash refuses;
//!   and always `trap`'s action, `alias`'s value, and the word after `builtin`, `busybox`,
//!   `function NAME` and `coproc`, which may not run), every word of the simple command is an
//!   argument of no known program: `sudo -iu postgres psql` is
//!   `sudo -<redacted> <redacted> <redacted>`.
//! - **The program word**: its name without a path or `.exe` ([`denylist::program_name`]:
//!   `/usr/bin/rm` is `rm`), kept when the word is literal and the name 1 to 64 of
//!   `[A-Za-z0-9._+-]` ([`allow::is_program`]); otherwise hidden. Wrappers are shown the same
//!   way.
//! - **Options**: `--` and `-` kept; a long option's name kept when it is 1 to 64 of
//!   `[A-Za-z0-9._-]`, its `=value` hidden (`--token=<redacted>`); a short option kept when it
//!   is one ASCII letter or digit, or a cluster of the program's flags that take no value
//!   (`rm -rf`, `git clean -fdx`), or one of the program's exact single-dash flags
//!   (`terraform -auto-approve`, `find -delete`); any other is `-<redacted>`, since its letters
//!   may hold a value (`mysql -psecret`). After a literal `--` every word up to the next
//!   wrapper is an operand, shown only as an argument is (`grep -- <redacted>`).
//! - **Arguments**: hidden, except a subcommand of the program (`git push`, `kubectl delete`,
//!   `docker system prune`), the key of a keyed operand (`dd of=`), and a sensitive target
//!   (`/`, `~`, `.`, `*`, `HEAD`, `main`, `origin`, `/dev/null`, `/dev/sda`, ...).
//! - **Redirections** at the end: `> <target>` for each file written (any write operator is
//!   shown as `>`), `< <redacted>` for a file read, `<<< <redacted>` for a heredoc or
//!   here-string; the target hidden unless it is a sensitive target. Comments are dropped.
//! - **Bodies**: a `bash -c`, `eval`, `su -c`, `script -c`, `flock -c` or `watch` (without
//!   `-x`) body (the denylist's [`denylist::Layout::Script`]) that is literal is lexed and
//!   rendered in turn, in `'…'`, at most [`MAX_NESTING`] deep and within the denylist's lexing
//!   budget; any other body, `trap`'s action and `alias`'s value among them, is hidden.
//!   `find`'s actions are rendered as commands, and its own words by its tables wherever the
//!   walk finds it (`sudo find`, `X=1 find`).
//!
//! Commands are joined by `; `, or ` | ` when a command reads the pipe of the one just before
//! it: the lexer keeps no other separator. A command the lexer cannot read is [`UNPARSED`],
//! nothing else. The skeleton is trimmed and cut to [`MAX_COMMAND_CHARS`].

mod allow;
mod certain;

use std::borrow::Cow;
use std::ops::Range;
use std::ptr;

use certain::End;

use super::denylist::{self, FindActions, Layout, Role};
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
            ParseError::Unclosed { .. }
            | ParseError::TooDeep
            | ParseError::MissingRedirectTarget
            | ParseError::ProcessInExtglob
            | ParseError::BacktickInExtglob
            | ParseError::ArithmeticInExtglob
            | ParseError::ExtglobName,
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
            if command.arithmetic {
                // Operands and operators of `$(( ))` or `(( ))`, not a program and arguments.
                line.word(REDACTED);
                continue;
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

    /// Renders one simple command's words, as the denylist lays them out, up to the first
    /// word whose place the layout does not give with certainty ([`certain`]); from there
    /// every word is an argument.
    fn words(&mut self, words: &[Word], line: &mut Line) {
        let (layout, roles) = denylist::layout(words);
        let all = Lead {
            words,
            roles: &roles,
        };
        let body = match &layout {
            Layout::Script(text) => Some(Body::of(words, text)),
            Layout::Program(_) | Layout::Exec(_) | Layout::Other => None,
        };
        let end = match (&layout, &body) {
            (Layout::Program(at), _) => End::Program(*at),
            (_, Some(body)) => body.end(),
            (Layout::Exec(exec), None) => End::Actions(&exec.actions),
            (Layout::Script(_) | Layout::Other, None) => End::Unchecked,
        };
        if let Some(cut) = certain::first_uncertain(words, &roles, end) {
            let (lead, rest) = all.split_at(cut);
            render_lead(lead, None, line);
            render_arguments(rest.words, lead.options(), line);
            return;
        }
        match (&layout, &body) {
            (Layout::Program(at), _) => render_program(all, *at, line),
            (_, Some(body)) => self.script(all, body, line),
            (Layout::Exec(exec), None) => self.exec(all, exec, line),
            (Layout::Script(_) | Layout::Other, None) => render_lead(all, None, line),
        }
    }

    /// A command whose wrappers run the shell text `body`: the words it came from become its
    /// skeleton in `'…'` (or [`REDACTED`] when they are not literal), the others are rendered
    /// as wrapper words.
    fn script(&mut self, all: Lead<'_>, body: &Body<'_>, line: &mut Line) {
        let Some(source) = &body.source else {
            // A body inside a word or built from several (`--command=body`, `alias`): the words
            // are rendered as they are, then the body after them when every word is literal.
            render_lead(all, None, line);
            if all.words.iter().all(Word::is_literal) {
                line.word(&quoted(&self.nested(body.text)));
            }
            return;
        };
        let (before, rest) = all.split_at(source.start);
        let (source, after) = rest.split_at(source.len());
        render_lead(before, None, line);
        if source.words.iter().all(Word::is_literal) {
            line.word(&quoted(&self.nested(body.text)));
        } else {
            line.word(REDACTED);
        }
        render_lead(after, None, line);
    }

    /// `find` with actions: its own words as wrapper words, by `find`'s tables wherever the
    /// walk found it, each action as a command one level deeper.
    fn exec(&mut self, all: Lead<'_>, exec: &FindActions, line: &mut Line) {
        let find = all.words.get(exec.find).and_then(named);
        // `rest` is `all` from index `at`; the actions come in order and do not overlap.
        let (mut rest, mut at) = (all, 0);
        for range in &exec.actions {
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

/// Words outside a program's arguments, and the role the denylist's walk gives each (as
/// `denylist::layout` returns them), index for index.
#[derive(Debug, Clone, Copy)]
struct Lead<'w> {
    words: &'w [Word],
    roles: &'w [Role],
}

impl Lead<'_> {
    /// The words before index `at` and from it; past the end, all of them and none.
    fn split_at(self, at: usize) -> (Self, Self) {
        let (words, words_after) = self.words.split_at_checked(at).unwrap_or((self.words, &[]));
        let (roles, roles_after) = self.roles.split_at_checked(at).unwrap_or((self.roles, &[]));
        (
            Self { words, roles },
            Self {
                words: words_after,
                roles: roles_after,
            },
        )
    }

    /// The options state [`render_lead`] ends these words in: ended when a literal `--` came
    /// among the words after the last wrapper.
    fn options(self) -> Options {
        let after = self
            .roles
            .iter()
            .rposition(|role| *role == Role::Wrapper)
            .map_or(0, |at| at.saturating_add(1));
        self.words
            .iter()
            .zip(self.roles)
            .skip(after)
            .filter(|(_, role)| **role == Role::Other)
            .fold(Options::Open, |options, (word, _)| options.after(word))
    }
}

/// The shell text a command's wrappers run, and where among its words it came from.
#[derive(Debug)]
struct Body<'b> {
    text: &'b str,
    /// The word or words it came from ([`body_source`]); `None` when it is inside one word
    /// or built from several.
    source: Option<Range<usize>>,
}

impl<'b> Body<'b> {
    /// The body `text` of the command `words`.
    fn of(words: &[Word], text: &'b str) -> Self {
        Self {
            text,
            source: body_source(words, text),
        }
    }

    /// Where the words of the wrapper running it end, for [`certain::first_uncertain`].
    fn end(&self) -> End<'b> {
        self.source
            .as_ref()
            .map_or(End::Attached, |source| End::Body {
                start: source.start,
                end: source.end,
            })
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

/// Renders words outside a program's arguments by the role the denylist's walk gives each:
/// keywords, assignments, wrappers, and their options, values and operands as arguments.
/// `program` names the wrapper before them, for its option tables.
fn render_lead(lead: Lead<'_>, program: Option<&str>, line: &mut Line) {
    let mut wrapper = program;
    let mut options = Options::Open;
    for (word, role) in lead.words.iter().zip(lead.roles) {
        match role {
            Role::Keyword if word.arithmetic => line.word(REDACTED),
            Role::Keyword => line.word(&word.text),
            Role::Assignment => line.word(&shown_assignment(word)),
            Role::Wrapper => {
                wrapper = named(word);
                options = Options::Open;
                line.word(&shown_program(word));
            }
            Role::Other => {
                line.word(&shown_argument(word, wrapper, options));
                options = options.after(word);
            }
        }
    }
}

/// Renders a command whose program is the word at `at`: the words before it as wrapper
/// words, the program by its name, its arguments by its tables.
fn render_program(all: Lead<'_>, at: usize, line: &mut Line) {
    let (lead, rest) = all.split_at(at);
    render_lead(lead, None, line);
    if let Some((program, args)) = rest.words.split_first() {
        line.word(&shown_program(program));
        let name = named(program);
        let mut options = Options::Open;
        for arg in args {
            line.word(&shown_argument(arg, name, options));
            options = options.after(arg);
        }
    }
}

/// Renders `words` as arguments of no known program, whatever role the walk gave them, from
/// the `options` state the words before them left: past the first word whose place is
/// uncertain ([`certain`]), a word may be a value as well as a program, wrapper or
/// assignment, so only the program-free tables may show it.
fn render_arguments(words: &[Word], mut options: Options, line: &mut Line) {
    for word in words {
        line.word(&shown_argument(word, None, options));
        options = options.after(word);
    }
}

/// Whether a program's options may still come, or a literal `--` ended them and every word
/// after it is an operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Options {
    /// A word starting with `-` is read as an option.
    Open,
    /// Every word is an operand.
    Ended,
}

impl Options {
    /// The state after the argument `word`.
    fn after(self, word: &Word) -> Self {
        if word.is_literal() && word.text == "--" {
            Self::Ended
        } else {
            self
        }
    }
}

/// The name a program or wrapper word runs, without a path or `.exe`
/// ([`denylist::program_name`]), for the tables: `None` when the word is not literal or is
/// part of an arithmetic expression (`$[ ]`).
fn named(word: &Word) -> Option<&str> {
    denylist::program_name(word).filter(|_| !word.arithmetic)
}

/// A program or wrapper word: its [`named`] name, when of [`allow::is_program`]'s shape.
fn shown_program(word: &Word) -> Cow<'_, str> {
    match named(word) {
        Some(name) if allow::is_program(name) => Cow::Borrowed(name),
        _ => Cow::Borrowed(REDACTED),
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

/// An argument of `program`: an option by the option rule while `options` are open, an
/// allowlisted word as written, anything else hidden.
fn shown_argument<'w>(word: &'w Word, program: Option<&str>, options: Options) -> Cow<'w, str> {
    let text = word.text.as_str();
    if word.shape == Shape::Dynamic || word.arithmetic {
        return Cow::Borrowed(REDACTED);
    }
    if options == Options::Open && word.is_literal() && text.starts_with('-') {
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
    if word.shape != Shape::Dynamic && !word.arithmetic && allow::is_target(&word.text) {
        Cow::Borrowed(&word.text)
    } else {
        Cow::Borrowed(REDACTED)
    }
}

#[cfg(test)]
mod tests;
