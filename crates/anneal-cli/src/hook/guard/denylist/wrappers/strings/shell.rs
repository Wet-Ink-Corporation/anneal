//! What a shell (`bash`, `sh`, `dash`, ...) runs given its arguments: its options, then a `-c`
//! string, a script file or a script read from standard input.
//!
//! A word known only at run time that may be an option cluster (`$o`, `"$y"-c`, `+$x`:
//! [`may_start_shell_options`]) is read both ways (#129): as the operand it is when its value is
//! not an option, and as options that may set `c` and `s`, take the next word as a value
//! (`-o NAME`) or end the options (`--`). So `bash $o 'rm -rf /'` (`o=-c`), `bash "$o" -e x`,
//! `bash "$o" pipefail -c x` (`o=-o`) and `bash -c "$x" 'rm -rf /'` (`x=-e`) have the string
//! read as the script they may run, as bash 5 and dash run it. A literal `+` cluster sets `c`
//! and `s` as a `-` one does (`bash +c x`, checked on both).
//!
//! The readings are walked as a set of states per word position, each the modes set so far
//! (four at most), so the walk is linear in the words however many may be options (H-13).

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
        let end = self.reach.len().saturating_sub(1);
        if let Some(slot) = self
            .reach
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
                    self.mark(next, widened);
                    self.mark(next.saturating_add(1), widened);
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
                let takes_value = if text.starts_with("--") {
                    SHELL_VALUES.contains(&text)
                } else {
                    text.ends_with(['o', 'O'])
                };
                let after = if takes_value {
                    next.saturating_add(1)
                } else {
                    next
                };
                self.mark(after, modes.after(text));
            }
            _ => self.end(at, modes),
        }
    }
}

/// What the shell `name` runs, given the arguments `words`: each reading's `-c` string, script
/// file or standard input, every one checked on its own.
pub(super) fn run<'a>(name: &'a str, words: &'a [Word]) -> Run<'a> {
    let mut walk = Walk::new(words.len());
    for at in 0..=words.len() {
        let Some(states) = walk.reach.get(at).copied() else {
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
            _ => {}
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
