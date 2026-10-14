//! Where the skeleton stops trusting the denylist's walk. The walk ([`denylist::layout`])
//! reads wrapper options loosely: an option it does not know is a flag, a short-option
//! cluster (`-iu`) is one flag, a value or operand is whatever word stands there, and chrt or
//! taskset with `-p` runs the words after the priority. That is the denylist's reading, kept
//! as the denylist reads it (see `denylist::wrappers`); where it differs from the wrapper's
//! own, the word it places as the program may be an option's value (`sudo -iu alice psql`
//! reads `alice` as the program; `chrt -i mysql -pX` reads `-pX`, since chrt takes no priority
//! there). So a wrapper's words are trusted only when they read with certainty by the tables
//! here ([`readings`]): each an exact flag (or, for a shell, a cluster of its flags), an exact
//! value option followed by its value (or with it attached, `-n1`, `--user=x`, where the
//! wrapper allows), `--`, then the wrapper's fixed operands and, for `sudo` and `env`,
//! literal assignments (bash field-splits a wrapper's arguments: `env K=$V x` may run `x` or
//! a word of `$V`), ending exactly where the walk put the next wrapper, the program or the
//! body, and a body only when no later word of the simple command starts with `-` or may
//! expand (`su` and `script` read options anywhere, the last `-c` winning).
//! A word filling a value or operand slot counts only when it is literal (no glob, variable
//! or substitution, which may become several words or none) and has that slot's strict shape
//! ([`shape::Shape`]: chrt's priority all digits, timeout's duration, a value not starting
//! with `-`); `find`'s words before its actions are read by [`find`]'s tables. `env -S` and
//! `watch -x` are not in the tables: both split their words and run them without a shell,
//! so what follows is no shell text the skeleton can render. From the first word that does
//! not read with certainty, the rest of the simple command is rendered as arguments: the
//! skeleton may hide more than needed, never show a value as a program.
//!
//! The tables trust only what they hold: a wrapper or option not listed is uncertain, and a
//! run of words is certain only where the tables and the walk read it the same way.

mod find;
mod readings;
mod shape;

use std::ops::Range;

use super::denylist::{self, Role};
use super::shell::Word;
use readings::{NONE, Reading, Style};
use shape::Shape;

/// Where the words of the last wrapper the walk read end, by the layout it gave the command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum End<'a> {
    /// At the program, this index.
    Program(usize),
    /// At the word or words the body came from, starting at this index.
    Body(usize),
    /// At the end of the command: the body is inside one of its words (`env -Sbody`,
    /// `--command=body`) or built from several (`alias`).
    Attached,
    /// At `find`'s actions, these index ranges in order: `find` is the last wrapper.
    Actions(&'a [Range<usize>]),
    /// Not checked: nothing after the last wrapper is a program or a body (a script file, a
    /// shell reading stdin, no command).
    Unchecked,
}

/// How the words of one wrapper must end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Exactly at the next wrapper or the program.
    Next,
    /// With an option of [`Reading::body`] (or anywhere, when the wrapper has none), the
    /// body being the next word.
    Body,
    /// At a word holding the body after an option of [`Reading::body`] (or anywhere, when the
    /// wrapper has none).
    Attached,
}

/// The index of the first word of `words` from which the walk's placing of the program or
/// body is not certain, or `None` when every wrapper's words read with certainty. `roles` are
/// the walk's, index for index; `end` says where the last wrapper's words end.
pub(super) fn first_uncertain(words: &[Word], roles: &[Role], end: End<'_>) -> Option<usize> {
    let wrappers: Vec<usize> = roles
        .iter()
        .enumerate()
        .filter(|(_, role)| **role == Role::Wrapper)
        .map(|(at, _)| at)
        .collect();
    for (nth, &at) in wrappers.iter().enumerate() {
        let start = at.saturating_add(1);
        let name = words.get(at).and_then(denylist::program_name);
        let (stop, mode) = match (wrappers.get(nth.saturating_add(1)), end) {
            (Some(&next), _) => (next, Mode::Next),
            (None, End::Program(program)) => (program, Mode::Next),
            (None, End::Body(body)) => (body, Mode::Body),
            (None, End::Attached) => (words.len(), Mode::Attached),
            (None, End::Actions(actions)) if name == Some("find") => {
                return find::first_uncertain(words, start, actions);
            }
            (None, End::Actions(_)) => return Some(start),
            (None, End::Unchecked) => return None,
        };
        let reading = name.and_then(readings::reading).unwrap_or(NONE);
        // Bounds the walk did not give in order are no certain reading.
        let (Some(run_words), Some(run_roles)) = (words.get(start..stop), roles.get(start..stop))
        else {
            return Some(start);
        };
        // The words after the body word, which the body's reading must also account for (none
        // when the body word is the last).
        let after = match mode {
            Mode::Body => words.get(stop.saturating_add(1)..).unwrap_or_default(),
            Mode::Next | Mode::Attached => &[],
        };
        let run = Run {
            words: run_words,
            roles: run_roles,
            after,
        };
        if let Some(offset) = run.first_uncertain(reading, mode) {
            return Some(start.saturating_add(offset));
        }
    }
    None
}

/// Whether the space-separated `list` holds `text`.
fn holds(list: &str, text: &str) -> bool {
    list.split_ascii_whitespace().any(|item| item == text)
}

/// Whether the space-separated `list` holds the short option of `letter` (`-c`).
fn holds_letter(list: &str, letter: char) -> bool {
    list.split_ascii_whitespace().any(|item| {
        item.strip_prefix('-')
            .is_some_and(|rest| rest.chars().eq([letter]))
    })
}

/// What an option word is to a wrapper, by its [`Reading`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Opt {
    /// An exact flag, or a shell's cluster of flags.
    Flag,
    /// An exact value option: its value is the next word, of this shape.
    Value(Shape),
    /// A value option with a value of its shape in the same word (`-uroot`, `--user=root`).
    Attached,
    /// An exact body option, or a shell's cluster of flags ending in one: the body is the
    /// next word.
    Body,
    /// A body option with the body in the same word (`-Sbody`, `--command=body`).
    AttachedBody,
    /// Anything else: an option the tables do not know, a cluster (`-iu`), or a value
    /// attached where the wrapper takes none or not of its option's shape.
    Unknown,
}

impl Reading {
    /// The syntax of the value of the value option `name`.
    fn shape(self, name: &str) -> Shape {
        self.shapes
            .iter()
            .find(|(option, _)| *option == name)
            .map_or(Shape::Any, |&(_, shape)| shape)
    }

    /// What the option word `text` is to this wrapper.
    fn option(self, text: &str) -> Opt {
        if holds(self.flags, text) {
            return Opt::Flag;
        }
        if holds(self.values, text) {
            return Opt::Value(self.shape(text));
        }
        if holds(self.body, text) {
            return Opt::Body;
        }
        if self.style == Style::Shell {
            return self.cluster(text);
        }
        // The option a value is attached to: `--name` before `=`, or `-` and the first letter.
        let named = match text.split_once('=') {
            Some((name, value)) if name.starts_with("--") => Some((name, value)),
            Some(_) => None,
            None if text.starts_with("--") => None,
            None => text
                .char_indices()
                .nth(2)
                .and_then(|(second, _)| text.split_at_checked(second)),
        };
        match named {
            Some((name, value)) if holds(self.values, name) => {
                if self.shape(name).accepts(value) {
                    Opt::Attached
                } else {
                    Opt::Unknown
                }
            }
            Some((name, _)) if holds(self.body, name) => Opt::AttachedBody,
            _ => Opt::Unknown,
        }
    }

    /// A shell's short-option cluster `text` (`-lc`): every letter one of its flags, a body
    /// option's letter only last.
    fn cluster(self, text: &str) -> Opt {
        let Some(letters) = text.strip_prefix('-').filter(|letters| {
            letters.len() > 1 && letters.bytes().all(|byte| byte.is_ascii_alphabetic())
        }) else {
            return Opt::Unknown;
        };
        let mut chars = letters.chars();
        let last = chars.next_back();
        if !chars.all(|letter| holds_letter(self.flags, letter)) {
            return Opt::Unknown;
        }
        match last {
            Some(letter) if holds_letter(self.flags, letter) => Opt::Flag,
            Some(letter) if holds_letter(self.body, letter) => Opt::Body,
            _ => Opt::Unknown,
        }
    }
}

/// The words between a wrapper and what it runs, with the walk's role for each.
#[derive(Debug, Clone, Copy)]
struct Run<'w> {
    words: &'w [Word],
    roles: &'w [Role],
    /// The words of the simple command after the body word, when the run ends at one.
    after: &'w [Word],
}

impl<'w> Run<'w> {
    /// The word at `at` when the walk gave it no role of its own (an option, a value or an
    /// operand); `None` past the end or for a keyword, assignment or wrapper.
    fn other(self, at: usize) -> Option<&'w Word> {
        match self.roles.get(at) {
            Some(Role::Other) => self.words.get(at),
            _ => None,
        }
    }

    /// The offset of the first word `reading` does not read with certainty, or `None` when
    /// every word does and the run ends as `mode` requires.
    fn first_uncertain(self, reading: Reading, mode: Mode) -> Option<usize> {
        let mut walk = Walk {
            reading,
            mode,
            operands: 0,
            options: true,
            listed: false,
        };
        let mut at = 0_usize;
        while let Some(word) = self.other(at) {
            let text = word.text.as_str();
            let step = if walk.options && word.is_literal() && text == "--" {
                walk.options = false;
                Step::To(at.saturating_add(1))
            } else if walk.is_option(word) {
                self.option(&mut walk, at, text)
            } else {
                walk.operand(at, word)
            };
            match step {
                Step::To(next) => at = next,
                Step::End => break,
                Step::Settled(first) => return first,
            }
        }
        self.end(&walk, at)
    }

    /// The option word `text` at `at`: a flag, a value option and its value, or a body.
    fn option(self, walk: &mut Walk, at: usize, text: &str) -> Step {
        let next = at.saturating_add(1);
        match walk.reading.option(text) {
            Opt::Flag => {
                walk.listed |= holds(walk.reading.lists, text);
                Step::To(next)
            }
            Opt::Attached => Step::To(next),
            Opt::Value(shape) => match self.other(next) {
                Some(value) if shape.fills(value) => Step::To(next.saturating_add(1)),
                Some(_) => Step::Settled(Some(next)),
                None => Step::Settled(Some(at)),
            },
            Opt::Body if walk.mode == Mode::Body && next == self.words.len() => self.body(at),
            Opt::AttachedBody if walk.mode == Mode::Attached => self.body(at),
            Opt::Body | Opt::AttachedBody | Opt::Unknown => Step::Settled(Some(at)),
        }
    }

    /// The body option at `at`, where the walk read the body: certain only when no later
    /// word of the simple command is option-shaped or may expand into one. `su` and `script`
    /// read options anywhere and the last `-c` wins, so after another option word the body
    /// the walk read may never run (`su -c 'x y' -c ls`).
    fn body(self, at: usize) -> Step {
        let later = self.words.get(at.saturating_add(1)..).unwrap_or_default();
        let unsure = later
            .iter()
            .chain(self.after)
            .any(|word| !word.is_literal() || word.text.starts_with('-'));
        Step::Settled(unsure.then_some(at))
    }

    /// Where the words after the options and operands, ending at `at`, leave the run: the
    /// wrapper's own assignments, then the end `mode` requires.
    fn end(self, walk: &Walk, mut at: usize) -> Option<usize> {
        if walk.operands < walk.reading.operands {
            return Some(at);
        }
        if walk.reading.assignments {
            while self.roles.get(at) == Some(&Role::Assignment) {
                // bash field-splits a wrapper's arguments, assignments included: `env K=$V x`
                // with `V="1 echo"` runs `echo`, so what follows a non-literal one is unsure.
                if !self.words.get(at).is_some_and(Word::is_literal) {
                    return Some(at);
                }
                at = at.saturating_add(1);
            }
        }
        // A wrapper with body options runs a body only through one of them.
        let unended = walk.mode != Mode::Next && !walk.reading.body.is_empty();
        (at < self.words.len() || unended).then_some(at)
    }
}

/// What reading one word of a [`Run`] does to the walk over it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Go on at this offset.
    To(usize),
    /// The options and operands end at this word.
    End,
    /// The run's reading is settled: the first uncertain offset, or `None` when certain.
    Settled(Option<usize>),
}

/// The state of the walk over one [`Run`] by its wrapper's [`Reading`].
#[derive(Debug, Clone, Copy)]
struct Walk {
    reading: Reading,
    mode: Mode,
    /// The operands read so far.
    operands: usize,
    /// Whether options may still follow (no `--` yet).
    options: bool,
    /// Whether a flag of [`Reading::lists`] was read.
    listed: bool,
}

impl Walk {
    /// Whether `word` stands as an option: literal, starting with `-`, and not a lone `-`
    /// unless the wrapper takes it as a flag (`su -`).
    fn is_option(&self, word: &Word) -> bool {
        let text = word.text.as_str();
        self.options
            && word.is_literal()
            && text.starts_with('-')
            && (text != "-" || holds(self.reading.flags, text))
    }

    /// The word at `at` that is no option: one of the wrapper's operands, of its shape, or
    /// where operands may stand among the options a literal one; else the options end.
    fn operand(&mut self, at: usize, word: &Word) -> Step {
        let next = at.saturating_add(1);
        if self.operands < self.reading.operands {
            let shape = if self.listed {
                Shape::CpuList
            } else {
                self.reading.operand
            };
            if !shape.fills(word) {
                return Step::Settled(Some(at));
            }
            self.operands = self.operands.saturating_add(1);
            return Step::To(next);
        }
        if !self.reading.operands_anywhere {
            return Step::End;
        }
        // A glob, variable or substitution may become an option or several words.
        if word.is_literal() {
            Step::To(next)
        } else {
            Step::Settled(Some(at))
        }
    }
}

#[cfg(test)]
mod tests;
