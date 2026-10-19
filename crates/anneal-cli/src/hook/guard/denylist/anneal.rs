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
//! have its required operands (`decide label "$id" yes`). A word that may become several words
//! (`"$@"`, `"${x[@]}"`, or any unquoted expansion, which the shell field-splits) may supply
//! any number of operands, so beside it another such word alone denies (`decide label $ARGS
//! "$flag"`). Arguments `xargs` appends at run time may be anything, so with them the path
//! alone denies. A call the guard cannot read ([`unread_call`]: a subcommand-path word not read
//! as written, `anneal "$@"` or `anneal $SUB`, which may stand for the whole path, or a
//! target's operands from a word that may become several, `anneal decide label "$@"`) is
//! denied when the call also holds both halves of a human actor ([`Halves`]: a literal
//! `--actor`, and a word that may be `human`, such as `"$(echo human)"`; or one word that may
//! be `--actor=human`), each anywhere, before or after it, in any simple command at any
//! nesting ([`Seen`]: `f() { anneal "$@"; }; f verdict set --actor human`, `bash -c 'anneal
//! decide label "$@"' _ d1 yes --actor "$(echo human)"`, `set -- d1 yes --actor; set -- "$@"
//! human; anneal decide label "$@"`). The program is anneal in any ASCII case, with or without
//! `.exe` (`ANNEAL`, `anneal.EXE`). A word that may become several words is otherwise judged
//! as one word: word splitting of an unquoted expansion into `--actor human` (`--sha $x`) is
//! the guard's documented limit (#128).

use crate::hook::guard::shell::Word;
use crate::hook::guard::{Denial, Rule};

use super::braces;

mod halves;

pub(super) use halves::Halves;

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
        || unread_path(args).is_some_and(|rest| Halves::of(rest).both());
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

/// Whether `anneal <args>` is a call the guard cannot read: its subcommand path is not read as
/// written ([`unread_path`]), or a target's path read as written is followed by a word that
/// may become several words ([`may_be_several`]: `anneal decide label "$@"`), which may supply
/// any operands and options, `--actor human` among them.
pub(super) fn unread_call(args: &[Word]) -> bool {
    unread_path(args).is_some() || several_after_path(args)
}

/// Whether `args` start with a target's path, read as written, followed by a word that may
/// become several words.
fn several_after_path(args: &[Word]) -> bool {
    let [command, sub, rest @ ..] = args else {
        return false;
    };
    let path_read = braces::reads_as_written(command)
        && braces::reads_as_written(sub)
        && TARGETS
            .iter()
            .any(|target| target.path == [command.text.as_str(), sub.text.as_str()]);
    path_read
        && rest
            .iter()
            .any(|word| !braces::reads_as_written(word) && may_be_several(word))
}

/// What a check has seen, so far, of an anneal call the guard cannot read ([`unread_call`])
/// and of the two halves of a human actor ([`Halves`]) anywhere in the call, in any order. The
/// actor may reach the call's arguments through words the guard does not follow (`"$@"` of a
/// function, a shell's positional arguments, `set --`, an array, one element at a time), so
/// the call and both halves together deny wherever they stand.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Seen {
    /// An anneal call the guard cannot read ([`unread_call`]).
    unread_call: bool,
    /// The halves of a human actor seen in any simple command of the call.
    actor: Halves,
}

impl Seen {
    /// Notes an anneal call the guard cannot read; the denial once both halves of a human
    /// actor were seen too.
    pub(super) fn call_unread(&mut self) -> Option<Denial> {
        self.unread_call = true;
        self.both()
    }

    /// Notes the halves of a human actor `words` hold; the denial once both halves and an
    /// unread anneal call were seen.
    pub(super) fn words(&mut self, words: &[Word]) -> Option<Denial> {
        self.actor = self.actor.union(Halves::of(words));
        self.both()
    }

    fn both(self) -> Option<Denial> {
        (self.unread_call && self.actor.both()).then(denial)
    }
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
    let mut count = Count::default();
    let mut words = rest.iter().peekable();
    while let Some(word) = words.next() {
        let next = words.peek().copied();
        if !braces::reads_as_written(word) {
            // Maybe an operand, maybe `--actor=human`, maybe `--actor` before `human`.
            let unsure = may_be(word, "--actor=human")
                || (may_be(word, ACTOR) && next.is_some_and(|value| may_be(value, HUMAN)));
            count.unread(word, unsure);
            continue;
        }
        let text = word.text.as_str();
        if text == "--" {
            count.operands = count.operands.saturating_add(words.count());
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
            count.operands = count.operands.saturating_add(1);
        } else if target.values.contains(&text)
            && next.is_some_and(|value| {
                !(braces::reads_as_written(value) && value.text.starts_with('-'))
            })
        {
            // Its value, which clap never reads as an option.
            words.next();
        }
    }
    count.may_claim(target.operands)
}

/// What [`claims_human`] counted of the words after a target's path.
#[derive(Debug, Default)]
struct Count {
    /// Operands, each word not read as written counted as one.
    operands: usize,
    /// Words not read as written that may become several words.
    several: usize,
    /// A word not read as written as one word may be the human actor.
    unsure_one: bool,
    /// A word that may become several words may be the human actor.
    unsure_several: bool,
}

impl Count {
    /// Counts `word`, not read as written, as one operand; `unsure` when it may be the human
    /// actor.
    fn unread(&mut self, word: &Word, unsure: bool) {
        self.operands = self.operands.saturating_add(1);
        if may_be_several(word) {
            self.several = self.several.saturating_add(1);
            self.unsure_several |= unsure;
        } else {
            self.unsure_one |= unsure;
        }
    }

    /// Whether an unsure word may be the human actor while the command still has its
    /// `required` operands: a word read as an option leaves one operand fewer, and a word
    /// that may become several words, other than the unsure one, may supply any number. A
    /// lone word of several words (`decide label "$@"`) is left to [`Seen`], which denies the
    /// call when it also holds both halves of a human actor ([`Halves`]).
    fn may_claim(&self, required: usize) -> bool {
        let counted = self.operands.saturating_sub(1) >= required;
        (self.unsure_one && (counted || self.several >= 1))
            || (self.unsure_several && (counted || self.several >= 2))
    }
}

/// Whether `word` may become several words: it holds an unquoted expansion, which the shell
/// field-splits (`$x`, `$*`, `${x[*]}`, `$(cmd)`), or a quoted one whose source text holds an
/// `@`, which may expand to one word per element: `"$@"`, `"${x[@]}"`, `"${!prefix@}"`, and
/// `$@` nested in another expansion (`"${x:-$@}"`, `"${x-"$@"}"`). A quoted `@` that gives one
/// word (`"${x@Q}"`) is counted too, which only denies more.
fn may_be_several(word: &Word) -> bool {
    word.expansions.iter().any(|expansion| {
        !expansion.quoted
            || word
                .text
                .get(expansion.range.clone())
                .is_none_or(|source| source.contains('@'))
    })
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
    text.starts_with(written_start(word))
}

/// What is written of `word` before its first expansion or pattern character: all of it when
/// it is read as written, and nothing when the cut is not on a character boundary (so the word
/// may be any text). A quoted pattern character is cut at too: the written start is then
/// shorter, which only lets the word be more texts.
fn written_start(word: &Word) -> &str {
    if braces::reads_as_written(word) {
        return &word.text;
    }
    let end = word
        .expansions
        .iter()
        .map(|expansion| expansion.range.start)
        .chain(word.text.find(PATTERN))
        .min()
        .unwrap_or(word.text.len());
    word.text.get(..end).unwrap_or_default()
}
