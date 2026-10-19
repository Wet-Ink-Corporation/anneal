//! `human-only` (#62): a Bash call may not run `anneal verdict set` or `anneal decide label`
//! with `--actor human`. The guard sees only the model's tool calls, and a human verdict or
//! label is a person's to give: from the cockpit mod (`/anneal`) or their own terminal.
//!
//! The words are judged as the shell forms them (the checker has removed wrappers, put in
//! variables and expanded braces), then as clap reads them: the subcommand path first, options
//! anywhere after it up to a `--`, each value-taking option's value the next word (`--sha
//! "$SHA"` is a value, never an option) or after `=`, and no abbreviated long options. A word
//! not read as written may be any text its written start allows, so `--actor "$who"` and a
//! lone `"$flags"` (which may be `--actor=human`) are denied, unless the command could then not
//! have its required operands (`decide label "$id" yes`). Arguments `xargs` appends at run time
//! may be anything, so with them the path alone denies. A subcommand-path word not read as
//! written (`anneal "$@"`, `anneal $SUB`) may stand for the whole path, so a literal
//! `--actor=human` or `--actor human` anywhere in the call denies, before or after it, in any
//! simple command at any nesting ([`Seen`]: `f() { anneal "$@"; }; f verdict set --actor
//! human`, `bash -c 'anneal "$@"' _ verdict set --actor human`, `set -- ... --actor human;
//! anneal "$@"`). The program is
//! anneal in any ASCII case, with or without `.exe` (`ANNEAL`, `anneal.EXE`). Each word is
//! judged as one word: word splitting of an unquoted expansion into `--actor human`
//! (`--sha $x`) is the guard's documented limit (#128).

use crate::hook::guard::shell::Word;
use crate::hook::guard::{Denial, Rule};

use super::braces;

/// The reason every `human-only` denial gives.
const REASON: &str = "human verdicts are human-only: use /anneal or run it in your own terminal";

/// The flag and value that claim a human.
const ACTOR: &str = "--actor";
const HUMAN: &str = "human";

/// A subcommand that records a human's word when given `--actor human`.
struct Target {
    /// Its path after `anneal`.
    path: [&'static str; 2],
    /// Its long options that take a value.
    values: &'static [&'static str],
    /// How many operands it requires.
    operands: usize,
}

const TARGETS: [Target; 2] = [
    Target {
        path: ["verdict", "set"],
        values: &[
            "--pr",
            "--sha",
            "--level",
            "--source",
            "--actor",
            "--via",
            "--base",
            "--session",
        ],
        operands: 0,
    },
    Target {
        path: ["decide", "label"],
        values: &["--actor", "--via"],
        operands: 2,
    },
];

/// The denial for `anneal <args>` when it may record a human verdict or label; `appended` when
/// `xargs` adds arguments known only at run time, which may be `--actor human`.
pub(super) fn check(args: &[Word], appended: bool) -> Option<Denial> {
    let claims = TARGETS
        .iter()
        .any(|target| claims_human(target, args, appended))
        || unread_path(args).is_some_and(names_human);
    claims.then(denial)
}

/// The `human-only` denial.
pub(super) fn denial() -> Denial {
    Denial::new(Rule::HumanOnly, REASON)
}

/// Whether the program name `name` (a last path component) is anneal: `anneal` in any ASCII
/// case, with or without `.exe` in any case, as a case-insensitive file system (APFS, NTFS)
/// finds it.
pub(super) fn is_anneal(name: &str) -> bool {
    let stem = name
        .len()
        .checked_sub(EXE.len())
        .and_then(|at| name.split_at_checked(at))
        .filter(|(_, extension)| extension.eq_ignore_ascii_case(EXE))
        .map_or(name, |(stem, _)| stem);
    stem.eq_ignore_ascii_case("anneal")
}

/// The extension a Windows program name may end in.
const EXE: &str = ".exe";

/// The words after the first subcommand-path word of `anneal <args>` that is not read as
/// written, when there is one (`anneal "$@"`, `anneal $SUB`, `anneal verdict "$@"`): such a
/// word may stand for any words, the whole path among them, so what follows it may be the
/// arguments of `verdict set` or `decide label`. `None` when the path is read as written as
/// far as it goes, or its first word is no target's (`anneal plan "$x"`).
pub(super) fn unread_path(args: &[Word]) -> Option<&[Word]> {
    match args {
        [first, rest @ ..] if !braces::reads_as_written(first) => Some(rest),
        [first, second, rest @ ..]
            if !braces::reads_as_written(second)
                && TARGETS
                    .iter()
                    .any(|target| matches!(target.path, [head, _] if head == first.text)) =>
        {
            Some(rest)
        }
        _ => None,
    }
}

/// What a check has seen, so far, of the two halves of a human actor passed to an anneal call
/// the guard cannot read: such a call ([`unread_path`]) and a literal human actor
/// ([`names_human`]) anywhere in the call, in either order. The actor may reach the call's
/// arguments through words the guard does not follow (`"$@"` of a function, a shell's
/// positional arguments, `set --`, an array), so the two together deny wherever they stand.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Seen {
    /// An anneal call had a subcommand-path word not read as written.
    unread_path: bool,
    /// A word read `--actor=human`, or two read `--actor human`.
    human: bool,
}

impl Seen {
    /// Notes an anneal call whose path is not read as written; the denial once a human actor
    /// was seen too.
    pub(super) fn path_unread(&mut self) -> Option<Denial> {
        self.unread_path = true;
        self.both()
    }

    /// Notes a literal human actor; the denial once an unread anneal path was seen too.
    pub(super) fn human_named(&mut self) -> Option<Denial> {
        self.human = true;
        self.both()
    }

    fn both(self) -> Option<Denial> {
        (self.unread_path && self.human).then(denial)
    }
}

/// Whether `words` hold, read as written, `--actor=human` or the pair `--actor human`.
pub(super) fn names_human(words: &[Word]) -> bool {
    let literal = |word: &Word, text: &str| braces::reads_as_written(word) && word.text == text;
    words.iter().any(|word| literal(word, "--actor=human"))
        || words.windows(2).any(
            |pair| matches!(pair, [flag, value] if literal(flag, ACTOR) && literal(value, HUMAN)),
        )
}

/// Whether the program `word` may be anneal: its last path component is anneal
/// ([`is_anneal`]), or is not known before the command runs.
pub(super) fn may_name_anneal(word: &Word) -> bool {
    let start = word
        .text
        .rfind(['/', '\\'])
        .map_or(0, |at| at.saturating_add(1));
    let Some(tail) = word.text.get(start..) else {
        return true;
    };
    let unknown = word
        .expansions
        .iter()
        .any(|expansion| expansion.range.end > start)
        || (!braces::reads_as_written(word) && tail.contains(PATTERN));
    unknown || is_anneal(tail)
}

/// Whether `args`, and any arguments `appended` at run time, may run `target` with
/// `--actor human`.
fn claims_human(target: &Target, args: &[Word], appended: bool) -> bool {
    let [first, second] = target.path;
    let (command, sub, rest) = match args {
        [command, sub, rest @ ..] => (command, sub, rest),
        // The path may be completed by what is appended, and then anything may follow it.
        [command] if appended => return may_be(command, first),
        [] => return appended,
        [_] => return false,
    };
    if !(may_be(command, first) && may_be(sub, second)) {
        return false;
    }
    if appended {
        return true;
    }
    let mut operands: usize = 0;
    let mut unsure = false;
    let mut words = rest.iter().peekable();
    while let Some(word) = words.next() {
        let next = words.peek().copied();
        if !braces::reads_as_written(word) {
            // Maybe an operand, maybe `--actor=human`, maybe `--actor` before `human`.
            operands = operands.saturating_add(1);
            unsure |= may_be(word, "--actor=human")
                || (may_be(word, ACTOR) && next.is_some_and(|value| may_be(value, HUMAN)));
            continue;
        }
        let text = word.text.as_str();
        if text == "--" {
            operands = operands.saturating_add(words.count());
            break;
        }
        if text == ACTOR && next.is_some_and(|value| may_be(value, HUMAN)) {
            return true;
        }
        if text
            .strip_prefix(ACTOR)
            .and_then(|rest| rest.strip_prefix('='))
            == Some(HUMAN)
        {
            return true;
        }
        if !text.starts_with('-') {
            operands = operands.saturating_add(1);
        } else if target.values.contains(&text)
            && next.is_some_and(|value| {
                !(braces::reads_as_written(value) && value.text.starts_with('-'))
            })
        {
            // Its value, which clap never reads as an option.
            words.next();
        }
    }
    // A word read as an option leaves one operand fewer for the command's required ones.
    unsure && operands.saturating_sub(1) >= target.operands
}

/// Characters after which a pattern word's text is no longer what it matches.
const PATTERN: [char; 9] = ['*', '?', '[', '{', '(', '~', '@', '+', '!'];

/// Whether `word` may be `text` once the shell has expanded it: exactly `text` when it is read
/// as written, else when `text` starts with what is written before its first expansion or
/// pattern character.
fn may_be(word: &Word, text: &str) -> bool {
    if braces::reads_as_written(word) {
        return word.text == text;
    }
    // A quoted pattern character is cut at too: the written start is then shorter, which only
    // lets the word be more texts.
    let end = word
        .expansions
        .iter()
        .map(|expansion| expansion.range.start)
        .chain(word.text.find(PATTERN))
        .min()
        .unwrap_or(word.text.len());
    word.text
        .get(..end)
        .is_none_or(|written| text.starts_with(written))
}
