//! What a shell (`bash`, `sh`, `dash`, ...) runs given its arguments: its options, then a `-c`
//! string, a script file or a script read from standard input.
//!
//! A name says neither which shell runs (`sh` may be dash, `busybox bash` is ash) nor whether
//! it has bash's long options, so every shell is read three ways ([`Phase`]): as bash reads
//! them (one dash or two, only before the first short option, #121), with none (every option
//! word a cluster, as dash, and bash after su's `-c` or `-f`, read them), and with a two-dash
//! long option wherever it stands (as read before #121). Each reading is checked.
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
//! su hands its shell `-c` and its command string first: a string that bash reads as an option
//! (`su -c -x root 'cmd'`) is read as the shell's first argument, with `c` already set.
//!
//! The readings are walked as a set of states per word position, each a reading's long
//! options and the modes it set so far (twelve at most), plus the states that reach every
//! position from one on, so the walk is linear in the words however many may be options
//! (H-13).

mod options;

use std::borrow::Cow;

use self::options::{Phase, Read, value_ends};
use super::script_file;
use crate::hook::guard::denylist::braces::reads_as_written;
use crate::hook::guard::denylist::readings::may_start_shell_options;
use crate::hook::guard::denylist::wrappers::Run;
use crate::hook::guard::shell::Word;

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

    /// This reading's slot in a phase's row.
    const fn slot(self) -> usize {
        match (self.command, self.stdin) {
            (false, false) => 0,
            (false, true) => 1,
            (true, false) => 2,
            (true, true) => 3,
        }
    }

    /// The modes after the literal cluster `text` (`-ec`, `+s`; `--norc` where no long option
    /// is read, whose `c` counts).
    fn after(self, text: &str) -> Self {
        Self {
            command: self.command || text.contains('c'),
            stdin: self.stdin || text.contains('s'),
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

/// One reading at a word position: the long options it reads and the modes it set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct State {
    phase: Phase,
    modes: Modes,
}

impl State {
    /// Every state, phase by phase.
    fn all() -> impl Iterator<Item = Self> {
        Phase::ALL
            .into_iter()
            .flat_map(|phase| Modes::ALL.map(|modes| Self { phase, modes }))
    }
}

/// Which states reach a position: a row per [`Phase`], a slot per [`Modes`].
type States = [[bool; 4]; 3];

/// Whether `state` is among `states`.
fn holds(states: &States, state: State) -> bool {
    states
        .get(state.phase.index())
        .and_then(|row| row.get(state.modes.slot()))
        .is_some_and(|reached| *reached)
}

/// One argument the shell reads: a word, or su's `-c` string, which su passes as written.
#[derive(Debug, Clone, Copy)]
enum Arg<'a> {
    Word(&'a Word),
    Lead(&'a str),
}

impl<'a> Arg<'a> {
    fn text(self) -> &'a str {
        match self {
            Self::Word(word) => &word.text,
            Self::Lead(text) => text,
        }
    }

    /// Whether the shell gets the argument as written.
    fn literal(self) -> bool {
        match self {
            Self::Word(word) => reads_as_written(word),
            Self::Lead(_) => true,
        }
    }

    /// Whether one of the argument's values may be an option cluster.
    fn may_be_options(self) -> bool {
        match self {
            Self::Word(word) => may_start_shell_options(word),
            Self::Lead(_) => false,
        }
    }
}

/// The states reaching each word position and what each reading that has left the options
/// runs, as flags per word so that none is listed twice.
#[derive(Debug)]
struct Walk {
    /// For each position, one past the last word included, which states reach it while the
    /// options are still being read.
    reach: Vec<States>,
    /// For each position, which states reach it and every later one while the options are
    /// still being read: a cluster known only at run time may take any number of values.
    reach_on: Vec<States>,
    /// The argument at this position is a reading's `-c` string.
    scripts: Vec<bool>,
    /// The argument at this position is a reading's script file.
    files: Vec<bool>,
    /// A reading reads its script from standard input.
    stdin: bool,
    /// A reading prints and runs nothing (`--help`, `--version`).
    exits: bool,
}

impl Walk {
    /// The walk of `args` arguments, every phase's reading at the first with `start` set.
    fn new(args: usize, start: Modes) -> Self {
        let mut walk = Self {
            reach: vec![[[false; 4]; 3]; args.saturating_add(1)],
            reach_on: vec![[[false; 4]; 3]; args.saturating_add(1)],
            scripts: vec![false; args],
            files: vec![false; args],
            stdin: false,
            exits: false,
        };
        for phase in Phase::ALL {
            walk.mark(
                0,
                State {
                    phase,
                    modes: start,
                },
            );
        }
        walk
    }

    /// A reading in `state` reaches position `at` still reading options; past the end (an
    /// option's missing value) it reaches the end, with no operand left.
    fn mark(&mut self, at: usize, state: State) {
        Self::set(&mut self.reach, at, state);
    }

    /// A reading in `state` reaches position `at` and every later one still reading options.
    fn mark_on(&mut self, at: usize, state: State) {
        Self::set(&mut self.reach_on, at, state);
    }

    /// Sets `state` at position `at` of `positions`, or at the end past it.
    fn set(positions: &mut [States], at: usize, state: State) {
        let end = positions.len().saturating_sub(1);
        if let Some(slot) = positions
            .get_mut(at.min(end))
            .and_then(|states| states.get_mut(state.phase.index()))
            .and_then(|row| row.get_mut(state.modes.slot()))
        {
            *slot = true;
        }
    }

    /// A reading with `modes` leaves the options with the argument at `at` as its first
    /// operand (none when `at` is past the last).
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

    /// Reads the argument `arg` at `at` in a reading in `state`.
    fn step(&mut self, at: usize, arg: Arg<'_>, state: State) {
        let next = at.saturating_add(1);
        let State { phase, modes } = state;
        if !arg.literal() {
            self.end(at, modes);
            if arg.may_be_options() {
                for &phase in phase.after_unknown() {
                    for modes in modes.widened() {
                        self.mark_on(next, State { phase, modes });
                        self.end(next, modes);
                    }
                }
            }
            return;
        }
        let text = arg.text();
        match Read::of(text, phase) {
            Read::Operand => self.end(at, modes),
            Read::Ends => self.end(next, modes),
            Read::Exits => self.exits = true,
            Read::Long(values) => self.mark(next.saturating_add(values), state),
            Read::Cluster => {
                let phase = phase.after_cluster();
                let set = modes.after(text);
                for after in value_ends(text).map(|values| next.saturating_add(values)) {
                    self.mark(after, State { phase, modes: set });
                    // A `+` cluster turns its options off in zsh, ksh and mksh.
                    if text.starts_with('+') {
                        self.mark(after, State { phase, modes });
                    }
                }
            }
        }
    }

    /// The states that reach position `at`: those marked there, and those that reach every
    /// position from an earlier one on, kept in `on` as the walk goes forward.
    fn states(&self, at: usize, on: &mut States) -> Option<States> {
        let mut states = *self.reach.get(at)?;
        let from = self.reach_on.get(at)?;
        for ((row, carried), starts) in states.iter_mut().zip(on.iter_mut()).zip(from) {
            for ((state, carried), starts) in row.iter_mut().zip(carried.iter_mut()).zip(starts) {
                *carried |= *starts;
                *state |= *carried;
            }
        }
        Some(states)
    }
}

/// What the shell `name` runs, given the arguments `words`: each reading's `-c` string, script
/// file or standard input, every one checked on its own. `lead`, when given, is the string of a
/// `-c` already read (su's `-c VALUE`), the shell's first argument after its `-c`.
pub(super) fn run<'a>(
    name: &'a str,
    lead: Option<&'a str>,
    words: impl IntoIterator<Item = &'a Word>,
) -> Run<'a> {
    let (args, walk, _) = walked(lead, words);
    let scripts = args
        .iter()
        .zip(&walk.scripts)
        .filter(|(_, script)| **script)
        .map(|(string, _)| Run::Script(Cow::Borrowed(string.text())));
    // A script file is one the guard does not read, stdin (`/dev/stdin`) or a descriptor.
    let (mut stdin, mut unseen, mut nothing) = (walk.stdin, false, walk.exits);
    for (arg, _) in args.iter().zip(&walk.files).filter(|(_, file)| **file) {
        // su's string is read with `c` set, so it is never a script file.
        let Arg::Word(script) = arg else {
            continue;
        };
        match script_file(name, script) {
            Run::StdinScript(_) => stdin = true,
            Run::Unseen(_) | Run::UnsureOption { .. } => unseen = true,
            // A file on disk: the guard does not read it, and it runs nothing to judge here.
            Run::Nothing
            | Run::Command { .. }
            | Run::Script(_)
            | Run::Split(_)
            | Run::Exec(_)
            | Run::Either(_) => nothing = true,
        }
    }
    let mut runs: Vec<Run<'a>> = scripts
        .chain(unseen.then_some(Run::Unseen(name)))
        .chain(stdin.then_some(Run::StdinScript(name)))
        .collect();
    match runs.len() {
        0 | 1 => runs.pop().unwrap_or(Run::Nothing),
        _ => Run::Either(runs),
    }
    .or_nothing(nothing)
}

/// How many of the shell's arguments `words` some reading still reads as options: up to and
/// including the last word a reading reaches while reading them, an option, its value or the
/// word that ends them (#165).
pub(super) fn option_words(words: &[Word]) -> usize {
    walked(None, words).2.map_or(0, |at| at.saturating_add(1))
}

/// The arguments `lead` and `words` and every reading's walk over them, with the last argument
/// a reading reaches while still reading options.
fn walked<'a>(
    lead: Option<&'a str>,
    words: impl IntoIterator<Item = &'a Word>,
) -> (Vec<Arg<'a>>, Walk, Option<usize>) {
    let args: Vec<Arg<'a>> = lead
        .map(Arg::Lead)
        .into_iter()
        .chain(words.into_iter().map(Arg::Word))
        .collect();
    let start = Modes {
        command: lead.is_some(),
        stdin: false,
    };
    let mut walk = Walk::new(args.len(), start);
    let mut on = [[false; 4]; 3];
    let mut last = None;
    for at in 0..=args.len() {
        let Some(states) = walk.states(at, &mut on) else {
            break;
        };
        for state in State::all().filter(|state| holds(&states, *state)) {
            match args.get(at) {
                Some(&arg) => {
                    last = Some(at);
                    walk.step(at, arg, state);
                }
                None => walk.end(at, state.modes),
            }
        }
    }
    (args, walk, last)
}

impl Run<'_> {
    /// This run, or, when a reading of the same arguments runs nothing (a script file on disk,
    /// `--version`) beside it, either. A missing `-c` string is not counted (`bash -c "$cmd"`
    /// stays one script); the redaction trusts no shell with two readings (`bash --login -c`).
    fn or_nothing(self, nothing: bool) -> Self {
        match self {
            Self::Nothing => Self::Nothing,
            Self::Either(mut runs) if nothing => {
                runs.push(Self::Nothing);
                Self::Either(runs)
            }
            run if nothing => Self::Either(vec![run, Self::Nothing]),
            run => run,
        }
    }
}
