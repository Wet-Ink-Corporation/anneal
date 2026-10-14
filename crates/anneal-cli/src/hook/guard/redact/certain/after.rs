//! What may follow a wrapper's body for the wrapper to run it: a word the wrapper does not
//! take there (a second file, a word it cannot read as a signal) makes it refuse the command
//! and run nothing, so the body's words are no program.

use super::shape::Shape;
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
    /// At least one signal, each literal and of [`Shape::Signal`], after an action neither
    /// all digits nor `-`: bash's `trap` reads such a first word, or a lone one, as signals
    /// to reset (`trap 'x y'` reads `x y` as a signal).
    Signals,
}

impl After {
    /// Whether a wrapper runs the body from the words `body`, with `following` after it and
    /// `before` operands read among its options before it.
    pub(super) fn admits<'w>(
        self,
        body: &[Word],
        following: impl Iterator<Item = &'w Word>,
        before: usize,
    ) -> bool {
        let mut following = following;
        let operand = |word: &Word| word.is_literal() && !word.text.starts_with('-');
        match self {
            Self::Operands => following.all(operand),
            Self::Nothing => following.next().is_none(),
            Self::AtMostOneOperand => following
                .try_fold(before, |operands, word| {
                    operand(word).then(|| operands.saturating_add(1))
                })
                .is_some_and(|operands| operands <= 1),
            Self::Signals => {
                let action = match body {
                    [action] => action.text != "-" && !Shape::Digits.accepts(&action.text),
                    _ => false,
                };
                let mut signals = following.peekable();
                action && signals.peek().is_some() && signals.all(|word| Shape::Signal.fills(word))
            }
        }
    }
}
