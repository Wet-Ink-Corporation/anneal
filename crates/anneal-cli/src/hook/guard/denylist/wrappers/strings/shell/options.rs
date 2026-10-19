//! What a literal word is among a shell's options, in each way the shell may read its long
//! options ([`Phase`]).

/// What one of bash's long options does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellLong {
    /// A flag.
    Flag,
    /// It takes the next word.
    Value,
    /// bash prints and runs nothing.
    Exits,
}

/// bash's long options (bash 5.2.21 as built by Debian and Ubuntu).
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

/// The long option `name` names, if bash has it.
fn long_option(name: &str) -> Option<ShellLong> {
    SHELL_LONG
        .iter()
        .find(|(long, _)| *long == name)
        .map(|&(_, option)| option)
}

/// Which long options a reading still reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    /// bash's, with one dash or two: no short option was read yet.
    Bash,
    /// None: every option word is a cluster.
    Clusters,
    /// Those written with two dashes, wherever they stand (`--norc`, `--rcfile FILE`), and
    /// any other `--` word as a flag, as read before #121.
    Anywhere,
}

impl Phase {
    /// Every phase, in [`Phase::index`] order.
    pub(super) const ALL: [Self; 3] = [Self::Bash, Self::Clusters, Self::Anywhere];

    /// This phase's row in a position's states.
    pub(super) const fn index(self) -> usize {
        match self {
            Self::Bash => 0,
            Self::Clusters => 1,
            Self::Anywhere => 2,
        }
    }

    /// The phase after a short cluster: bash reads no long option after one.
    pub(super) const fn after_cluster(self) -> Self {
        match self {
            Self::Bash | Self::Clusters => Self::Clusters,
            Self::Anywhere => Self::Anywhere,
        }
    }

    /// The phases after a word known only at run time read as options: in bash's it may be a
    /// long option or a short cluster.
    pub(super) const fn after_unknown(self) -> &'static [Self] {
        match self {
            Self::Bash => &[Self::Bash, Self::Clusters],
            Self::Clusters => &[Self::Clusters],
            Self::Anywhere => &[Self::Anywhere],
        }
    }
}

/// What a literal argument is to a reading in a phase.
pub(super) enum Read {
    /// The first operand.
    Operand,
    /// `--` or `-`: the options end after it.
    Ends,
    /// `--help` or `--version`: the shell runs nothing.
    Exits,
    /// A long option taking this many words.
    Long(usize),
    /// A short cluster (`-ec`, `+o`).
    Cluster,
}

impl Read {
    /// What the literal argument `text` is in `phase`.
    pub(super) fn of(text: &str, phase: Phase) -> Self {
        if text == "--" || text == "-" {
            return Self::Ends;
        }
        let long = match phase {
            Phase::Bash => text
                .strip_prefix("--")
                .or_else(|| text.strip_prefix('-'))
                .and_then(long_option),
            // Any other two-dash word is a flag in this reading.
            Phase::Anywhere => text
                .strip_prefix("--")
                .map(|name| long_option(name).unwrap_or(ShellLong::Flag)),
            Phase::Clusters => None,
        };
        match long {
            Some(ShellLong::Flag) => Self::Long(0),
            Some(ShellLong::Value) => Self::Long(1),
            Some(ShellLong::Exits) => Self::Exits,
            None if text.starts_with(['-', '+']) => Self::Cluster,
            None => Self::Operand,
        }
    }
}

/// How many words the literal cluster `text` may take as values, each a reading: one per `o`
/// or `O` letter, as bash reads `-oc posix` and `-coo posix pipefail`, and, as read before
/// #129, one only when it ends in such a letter.
pub(super) fn value_ends(text: &str) -> [usize; 2] {
    let letters = text
        .chars()
        .skip(1)
        .filter(|c| matches!(c, 'o' | 'O'))
        .count();
    [letters, usize::from(text.ends_with(['o', 'O']))]
}
