//! The Tier 1 scorer: a skill's score for a prompt is the number of distinct content-word stems
//! they share.
//!
//! Content words are those of `lint skills`' overlap check (lowercase, three letters or more,
//! stopwords removed), each cut to a stem by [`stem`] so `plan`, `plans` and `planning` match.

use std::collections::BTreeSet;

use crate::eval::routing::Top;
use crate::lint::skills::vocabulary;

/// Suffixes [`stem`] removes, longest first. One is removed at most.
const SUFFIXES: [&str; 4] = ["ing", "ed", "es", "s"];

/// The shortest stem left after removing a suffix, a final `e` or a doubled letter.
const MIN_STEM: usize = 3;

/// A light suffix stemmer: removes one of [`SUFFIXES`] (not the `s` of `ss`), undoubles a
/// final consonant left by `-ing` or `-ed` (`planned` to `plan`, but `spelled` keeps its `ll`),
/// then drops a final `e`, never leaving fewer than [`MIN_STEM`] characters.
pub(crate) fn stem(word: &str) -> String {
    let long_enough = |base: &str| base.chars().count() >= MIN_STEM;
    let removed = SUFFIXES.iter().find_map(|&suffix| {
        word.strip_suffix(suffix)
            .filter(|base| long_enough(base) && !(suffix == "s" && base.ends_with('s')))
            .map(|base| (base, suffix))
    });
    let mut base = removed.map_or(word, |(base, _)| base);
    if removed.is_some_and(|(_, suffix)| suffix == "ing" || suffix == "ed") {
        base = undouble(base).filter(|b| long_enough(b)).unwrap_or(base);
    }
    base.strip_suffix('e')
        .filter(|b| long_enough(b))
        .unwrap_or(base)
        .to_owned()
}

/// `base` without its last letter when it ends in a doubled consonant other than `l`, `s` or
/// `z`.
fn undouble(base: &str) -> Option<&str> {
    let mut last = base.chars().rev();
    let (Some(a), Some(b)) = (last.next(), last.next()) else {
        return None;
    };
    let doubled = a == b
        && a.is_ascii_lowercase()
        && !matches!(a, 'a' | 'e' | 'i' | 'o' | 'u' | 'l' | 's' | 'z');
    doubled.then(|| base.strip_suffix(a)).flatten()
}

/// The distinct content-word stems of `text`.
pub(crate) fn stems(text: &str) -> BTreeSet<String> {
    vocabulary(text).iter().map(|word| stem(word)).collect()
}

/// Scores prompts against a fixed set of skills.
#[derive(Debug)]
pub(crate) struct Scorer<'s> {
    skills: Vec<(&'s str, BTreeSet<String>)>,
}

impl<'s> Scorer<'s> {
    /// A scorer for `skills`, each a name and the text the router sees for it.
    pub(crate) fn new<'t>(skills: impl IntoIterator<Item = (&'s str, &'t str)>) -> Self {
        Self {
            skills: skills
                .into_iter()
                .map(|(name, text)| (name, stems(text)))
                .collect(),
        }
    }

    /// True when `skill` is one of the scored skills.
    pub(crate) fn has(&self, skill: &str) -> bool {
        self.skills.iter().any(|(name, _)| *name == skill)
    }

    /// Every skill's score for `prompt`, in the order the skills were given.
    pub(crate) fn score(&self, prompt: &str) -> Scores<'s> {
        let prompt = stems(prompt);
        Scores {
            scores: self
                .skills
                .iter()
                .map(|(name, words)| (*name, words.intersection(&prompt).count()))
                .collect(),
        }
    }
}

/// The skills' scores for one prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Scores<'s> {
    scores: Vec<(&'s str, usize)>,
}

impl Scores<'_> {
    /// The score of `skill`, or 0 when it is not scored.
    pub(crate) fn of(&self, skill: &str) -> usize {
        self.scores
            .iter()
            .find(|(name, _)| *name == skill)
            .map_or(0, |(_, score)| *score)
    }

    /// The unique best skill, [`Top::Tie`] when several share the best score, or
    /// [`Top::NoMatch`] when no skill shares a word with the prompt.
    pub(crate) fn top(&self) -> Top {
        let best = self
            .scores
            .iter()
            .map(|(_, score)| *score)
            .max()
            .unwrap_or(0);
        let mut leaders = self.scores.iter().filter(|(_, score)| *score == best);
        match (best, leaders.next(), leaders.next()) {
            (0, _, _) | (_, None, _) => Top::NoMatch,
            (_, Some(_), Some(_)) => Top::Tie,
            (_, Some((name, _)), None) => Top::Skill((*name).to_owned()),
        }
    }

    /// Every scored skill and its score.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, usize)> + '_ {
        self.scores.iter().map(|(name, score)| (*name, *score))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{Scorer, stem, stems};
    use crate::eval::routing::Top;

    #[test]
    fn inflections_share_a_stem() {
        let groups = [
            ["plan", "plans", "planned", "planning"],
            ["release", "releases", "released", "releasing"],
            ["spell", "spells", "spelled", "spelling"],
            ["fix", "fixes", "fixed", "fixing"],
        ];
        for group in groups {
            let found: BTreeSet<String> = group.iter().map(|w| stem(w)).collect();
            assert_eq!(found.len(), 1, "{group:?} stem to one word: {found:?}");
        }
    }

    #[test]
    fn stems_drop_stopwords_and_short_words() {
        assert_eq!(
            stems("Use it when planning the tasks"),
            BTreeSet::from(["plan".to_owned(), "task".to_owned()]),
            "`use`, `it`, `when`, `the` carry no signal"
        );
    }

    #[test]
    fn a_skill_scores_the_stems_it_shares_with_the_prompt() {
        let scorer = Scorer::new([
            ("plan", "plan: break a spec into ordered tasks"),
            ("spec", "spec: write the requirements of a feature"),
        ]);

        let ranking = scorer.score("break the feature into tasks");

        assert_eq!(ranking.of("plan"), 2, "break, task");
        assert_eq!(ranking.of("spec"), 1, "feature");
        assert_eq!(ranking.of("missing"), 0, "an unknown skill scores 0");
        assert_eq!(ranking.top(), Top::Skill("plan".to_owned()), "unique best");
    }

    #[test]
    fn top_is_a_tie_for_shared_best_and_none_for_no_overlap() {
        let scorer = Scorer::new([("plan", "plan tasks"), ("spec", "spec tasks")]);

        assert_eq!(scorer.score("list the tasks").top(), Top::Tie, "both 1");
        assert_eq!(scorer.score("deploy it").top(), Top::NoMatch, "both 0");
    }
}
