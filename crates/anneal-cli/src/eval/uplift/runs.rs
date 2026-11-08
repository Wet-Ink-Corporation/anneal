//! How many times each case runs in each arm (`--runs`).

/// Runs per case in each arm: [`Runs::MIN`] to [`Runs::MAX`] (`--runs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Runs(u32);

impl Runs {
    /// The fewest runs a case may be asked for.
    pub(crate) const MIN: u32 = 3;
    /// The most runs a case may be asked for.
    pub(crate) const MAX: u32 = 20;
    /// A round's runs, the default.
    pub(crate) const ROUND: Self = Self(3);
    /// A guard confirmation's runs, as the confirmation command asks for them.
    pub(crate) const CONFIRMATION: Self = Self(10);

    /// `n` runs, when `n` is within [`Runs::MIN`]`..=`[`Runs::MAX`].
    pub(crate) const fn new(n: u32) -> Option<Self> {
        if n >= Self::MIN && n <= Self::MAX {
            Some(Self(n))
        } else {
            None
        }
    }

    /// The number of runs.
    pub(crate) const fn get(self) -> u32 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::Runs;

    #[test]
    fn runs_are_3_to_20_with_a_round_of_3_and_a_confirmation_of_10() {
        assert_eq!(
            [2, 3, 20, 21].map(|n| Runs::new(n).map(Runs::get)),
            [None, Some(3), Some(20), None]
        );
        assert_eq!((Runs::ROUND.get(), Runs::CONFIRMATION.get()), (3, 10));
    }
}
