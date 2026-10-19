//! What a shell (`bash`, `sh`, `dash`, ...) runs given its arguments: its options, then a `-c`
//! string, a script file or a script read from standard input.
//!
//! A word known only at run time that may be an option cluster (`$o`, `"$y"-c`, `+$x`:
//! [`may_start_shell_options`]) is read both ways (#129): as the operand it is when its value is
//! not an option, and as options that may set `c` and `s`, take any number of later words as
//! values (`-oo NAME NAME`) or end the options (`--`). So `bash $o 'rm -rf /'` (`o=-c`),
//! `bash "$o" -e x`, `bash "$o" pipefail -c x` (`o=-o`) and `bash -c "$x" 'rm -rf /'`
//! (`x=-e`) have the string read as the script they may run, as bash 5 and dash run it.
//!
//! A literal short cluster takes one value per `o` or `O` letter wherever it stands, as bash
//! reads `bash -oc posix '...'`; the reading before #129, where only a trailing `o` takes one,
//! is kept beside it. A literal `+` cluster is read both ways: setting `c` and `s` as a `-` one
//! does (`bash +c x`, as bash and dash run it) and setting nothing, so the next word is the
//! script file (`zsh +s <(...)`, as zsh, ksh and mksh may read it).
//!
//! The readings are walked as a set of states per word position, each the modes set so far
//! (four at most), plus the modes that reach every position from one on, so the walk is
//! linear in the words however many may be options (H-13).

use std::borrow::Cow;

use super::script_file;
use crate::hook::guard::denylist::braces::reads_as_written;
use crate::hook::guard::denylist::readings::may_start_shell_options;
use crate::hook::guard::denylist::wrappers::Run;
use crate::hook::guard::shell::Word;

/// Bash's long options that take the next word as their value.
const SHELL_VALUES: &[&str] = &["--rcfile", "--init-file"];

/// What one reading of a shell's options has set so far: `c` (the first operand is the
/// command string) and `s` (the script is standard input).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Modes {
    command: bool,
    stdin: bool,
}

impl Modes {
    const NONE: Self = Self {
        command: false,
        stdin: false,
    };

    /// Every set of modes, in [`Modes::slot`] order.
    const ALL: [Self; 4] = [
        Self::NONE,
        Self {
            command: false,
            stdin: true,
        },
        Self {
            command: true,
            stdin: false,
        },
        Self {
            command: true,
            stdin: true,
        },
    ];

    /// This reading's slot in a position's states.
    const fn slot(self) -> usize {
        match (self.command, self.stdin) {
            (false, false) => 0,
            (false, true) => 1,
            (true, false) => 2,
            (true, true) => 3,
        }
    }

    /// The modes after the literal cluster `text` (`-ec`, `+s`, `--norc`).
    fn after(self, text: &str) -> Self {
        let short = !text.starts_with("--");
        Self {
            command: self.command || (short && text.contains('c')),
            stdin: self.stdin || (short && text.contains('s')),
        }
    }

    /// Every set of modes a cluster known only at run time may leave: these, with `c` and `s`
    /// each set or not.
    fn widened(self) -> [Self; 4] {
        Self::ALL.map(|set| Self {
            command: self.command || set.command,
            stdin: self.stdin || set.stdin,
        })
    }
}

/// The states reaching each word position, by [`Modes::slot`], and what each reading that has
/// left the options runs, as flags per word so that none is listed twice.
#[derive(Debug)]
struct Walk {
    /// For each position, one past the last word included, which modes reach it while the
    /// options are still being read.
    reach: Vec<[bool; 4]>,
    /// For each position, which modes reach it and every later one while the options are
    /// still being read: a cluster known only at run time may take any number of values.
    reach_on: Vec<[bool; 4]>,
    /// The word at this position is a reading's `-c` string.
    scripts: Vec<bool>,
    /// The word at this position is a reading's script file.
    files: Vec<bool>,
    /// A reading reads its script from standard input.
    stdin: bool,
}

impl Walk {
    /// The walk of `words` words, one reading at the first with no mode set.
    fn new(words: usize) -> Self {
        let mut walk = Self {
            reach: vec![[false; 4]; words.saturating_add(1)],
            reach_on: vec![[false; 4]; words.saturating_add(1)],
            scripts: vec![false; words],
            files: vec![false; words],
            stdin: false,
        };
        walk.mark(0, Modes::NONE);
        walk
    }

    /// A reading with `modes` reaches position `at` still reading options; past the end (an
    /// option's missing value) it reaches the end, with no operand left.
    fn mark(&mut self, at: usize, modes: Modes) {
        Self::set(&mut self.reach, at, modes);
    }

    /// A reading with `modes` reaches position `at` and every later one still reading options.
    fn mark_on(&mut self, at: usize, modes: Modes) {
        Self::set(&mut self.reach_on, at, modes);
    }

    /// Sets `modes` at position `at` of `states`, or at the end past it.
    fn set(states: &mut [[bool; 4]], at: usize, modes: Modes) {
        let end = states.len().saturating_sub(1);
        if let Some(slot) = states
            .get_mut(at.min(end))
            .and_then(|states| states.get_mut(modes.slot()))
        {
            *slot = true;
        }
    }

    /// A reading with `modes` leaves the options with the word at `at` as its first operand
    /// (none when `at` is past the last word).
    fn end(&mut self, at: usize, modes: Modes) {
        let flags = if modes.command {
            &mut self.scripts
        } else if modes.stdin {
            self.stdin = true;
            return;
        } else {
            &mut self.files
        };
        match flags.get_mut(at) {
            Some(flag) => *flag = true,
            // `-c` with no string runs nothing; no operand at all reads stdin.
            None => self.stdin |= !modes.command,
        }
    }

    /// Reads the word `word` at `at` in a reading with `modes`.
    fn step(&mut self, at: usize, word: &Word, modes: Modes) {
        let next = at.saturating_add(1);
        let text = word.text.as_str();
        if !reads_as_written(word) {
            self.end(at, modes);
            if may_start_shell_options(word) {
                for widened in modes.widened() {
                    self.mark_on(next, widened);
                    self.end(next, widened);
                }
            }
            return;
        }
        match text {
            // This reading prints and runs nothing.
            "--version" | "--help" => {}
            "--" | "-" => self.end(next, modes),
            _ if text.starts_with(['-', '+']) => {
                let set = modes.after(text);
                for after in value_ends(text).map(|values| next.saturating_add(values)) {
                    self.mark(after, set);
                    // A `+` cluster turns its options off in zsh, ksh and mksh.
                    if text.starts_with('+') {
                        self.mark(after, modes);
                    }
                }
            }
            _ => self.end(at, modes),
        }
    }

    /// The modes that reach position `at`: those marked there, and those that reach every
    /// position from an earlier one on, kept in `on` as the walk goes forward.
    fn states(&self, at: usize, on: &mut [bool; 4]) -> Option<[bool; 4]> {
        let mut states = *self.reach.get(at)?;
        let from = self.reach_on.get(at)?;
        for ((state, carried), starts) in states.iter_mut().zip(on.iter_mut()).zip(from) {
            *carried |= *starts;
            *state |= *carried;
        }
        Some(states)
    }
}

/// How many words the literal cluster `text` may take as values, each a reading: a long
/// option of [`SHELL_VALUES`] takes one; a short cluster one per `o` or `O` letter, as bash
/// reads `-oc posix` and `-coo posix pipefail`, and, as read before #129, one only when it
/// ends in such a letter.
fn value_ends(text: &str) -> [usize; 2] {
    if text.starts_with("--") {
        let values = usize::from(SHELL_VALUES.contains(&text));
        return [values, values];
    }
    let letters = text
        .chars()
        .skip(1)
        .filter(|c| matches!(c, 'o' | 'O'))
        .count();
    [letters, usize::from(text.ends_with(['o', 'O']))]
}

/// What the shell `name` runs, given the arguments `words`: each reading's `-c` string, script
/// file or standard input, every one checked on its own.
pub(super) fn run<'a>(name: &'a str, words: &'a [Word]) -> Run<'a> {
    let mut walk = Walk::new(words.len());
    let mut on = [false; 4];
    for at in 0..=words.len() {
        let Some(states) = walk.states(at, &mut on) else {
            break;
        };
        for (modes, reached) in Modes::ALL.into_iter().zip(states) {
            if !reached {
                continue;
            }
            match words.get(at) {
                Some(word) => walk.step(at, word, modes),
                None => walk.end(at, modes),
            }
        }
    }
    let scripts = words
        .iter()
        .zip(&walk.scripts)
        .filter(|(_, script)| **script)
        .map(|(string, _)| Run::Script(Cow::Borrowed(string.text.as_str())));
    // A script file is one the guard does not read, stdin (`/dev/stdin`) or a descriptor.
    let (mut stdin, mut unseen) = (walk.stdin, false);
    for (script, _) in words.iter().zip(&walk.files).filter(|(_, file)| **file) {
        match script_file(name, script) {
            Run::StdinScript(_) => stdin = true,
            Run::Unseen(_) => unseen = true,
            Run::Nothing | Run::Command { .. } | Run::Script(_) | Run::Exec(_) | Run::Either(_) => {
                // A file on disk: the guard does not read it, and it runs nothing to judge here.
            }
        }
    }
    let mut runs: Vec<Run<'a>> = scripts
        .chain(unseen.then_some(Run::Unseen(name)))
        .chain(stdin.then_some(Run::StdinScript(name)))
        .collect();
    if runs.len() > 1 {
        Run::Either(runs)
    } else {
        runs.pop().unwrap_or(Run::Nothing)
    }
}
