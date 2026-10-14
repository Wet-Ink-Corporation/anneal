//! What may follow a wrapper's body for the wrapper to run it: a word the wrapper does not
//! take there (a second file, an option word) makes it refuse the command and run nothing, so
//! the body's words are no program. (`trap` and `alias`, which may never run their body, are
//! not read here: their [`super::readings::Runs`] cuts at the body.)

use crate::hook::guard::shell::Word;

/// What may follow the word or words a wrapper's body comes from, for the wrapper to run that
/// body. Every rule refuses a word that starts with `-` or may expand into one: `su` and
/// `script` read options anywhere and the last `-c` wins (`su -c 'x y' -c ls`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum After {
    /// Words the wrapper passes on (a shell's `$0` and arguments, `su`'s user and arguments),
    /// each literal and not starting with `-`.
    Operands,
    /// Nothing: `flock` takes exactly one word after `-c` (it exits 64 otherwise).
    Nothing,
    /// At most one operand in the whole run, before the body or after it, literal and not
    /// starting with `-`: `script`'s file (`script -c x a b` exits 1).
    AtMostOneOperand,
}

impl After {
    /// Whether a wrapper runs its body with `following` after it and `before` operands read
    /// among its options before it.
    pub(super) fn admits<'w>(
        self,
        mut following: impl Iterator<Item = &'w Word>,
        before: usize,
    ) -> bool {
        let operand = |word: &Word| word.is_literal() && !word.text.starts_with('-');
        match self {
            Self::Operands => following.all(operand),
            Self::Nothing => following.next().is_none(),
            Self::AtMostOneOperand => following
                .try_fold(before, |operands, word| {
                    operand(word).then(|| operands.saturating_add(1))
                })
                .is_some_and(|operands| operands <= 1),
        }
    }
}
