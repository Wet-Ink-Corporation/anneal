//! Judging fixtures against skills: pure, no I/O.

use std::collections::BTreeSet;

use crate::eval::routing::fixture::{self, Fixture};
use crate::eval::routing::score::Scorer;
use crate::eval::routing::{
    Case, CaseKind, Fault, List, MAX_PROMPTS, MIN_PROMPTS, Outcome, Problem, Rival, Source, Top,
};
use crate::lint::skills::listing::{Listing, Unparsed};

/// Scores `sources` against the model-invoked skills among `skills`.
pub(crate) fn judge(skills: Vec<Result<Listing, Unparsed>>, sources: Vec<Source>) -> Outcome {
    let mut problems = Vec::new();
    let mut listed = Vec::with_capacity(skills.len());
    let mut unparsed = BTreeSet::new();
    for skill in skills {
        match skill {
            Ok(listing) => listed.push(listing),
            Err(skill) => {
                unparsed.insert(skill.name);
                problems.push(Problem::Skill(skill.finding));
            }
        }
    }
    let routable: Vec<&Listing> = listed.iter().filter(|l| l.model_invoked).collect();
    let roster = Roster {
        listed: &listed,
        unparsed: &unparsed,
        scorer: Scorer::new(routable.iter().map(|l| (l.name.as_str(), l.text.as_str()))),
    };
    let mut cases = Vec::new();
    let mut covered = BTreeSet::new();
    for source in sources {
        let judged = judge_fixture(source, &roster);
        covered.extend(judged.covers);
        cases.extend(judged.cases);
        problems.extend(judged.problems);
    }
    let mut names: Vec<String> = listed
        .into_iter()
        .filter(|l| l.model_invoked)
        .map(|l| l.name)
        .collect();
    names.sort();
    problems.extend(
        names
            .iter()
            .filter(|name| !covered.contains(name.as_str()))
            .map(|skill| Problem::MissingFixture {
                // The name is both in `skills` and in this problem: genuine duplication (H-08).
                skill: skill.clone(),
            }),
    );
    Outcome {
        skills: names,
        cases,
        problems,
    }
}

/// What one fixture contributes.
#[derive(Debug, Default)]
struct Judged {
    /// The skill it covers, so that skill is not reported as missing a fixture.
    covers: Option<String>,
    cases: Vec<Case>,
    problems: Vec<Problem>,
}

/// The skill set a fixture is judged against.
#[derive(Debug)]
struct Roster<'s> {
    /// Every skill whose `SKILL.md` parsed.
    listed: &'s [Listing],
    /// The names of skills whose `SKILL.md` did not parse.
    unparsed: &'s BTreeSet<String>,
    /// Scores prompts against the model-invoked skills.
    scorer: Scorer<'s>,
}

impl Roster<'_> {
    /// Why `owner` cannot own a near miss of `skill`'s fixture, if it cannot.
    fn owner_fault(&self, owner: &str, skill: &str) -> Option<Fault> {
        if owner == skill {
            Some(Fault::SelfOwned)
        } else if self.unparsed.contains(owner) {
            Some(Fault::OwnerUnparseable {
                owner: owner.to_owned(),
            })
        } else if self.scorer.has(owner) {
            None
        } else {
            Some(Fault::UnknownOwner {
                owner: owner.to_owned(),
            })
        }
    }

    /// Why no fixture can route to `skill`, if none can.
    fn skill_fault(&self, skill: &str) -> Option<Fault> {
        if self.unparsed.contains(skill) {
            return Some(Fault::SkillUnparseable);
        }
        match self.listed.iter().find(|l| l.name == skill) {
            None => Some(Fault::UnknownSkill),
            Some(listing) if !listing.model_invoked => Some(Fault::NotRoutable),
            Some(_) => None,
        }
    }
}

fn judge_fixture(source: Source, roster: &Roster<'_>) -> Judged {
    let file = source.file.as_str();
    let problem = |line, fault| Problem::Fixture {
        file: file.to_owned(),
        line,
        fault,
    };
    let Some(skill) = source.skill.as_deref() else {
        return Judged {
            problems: vec![problem(0, Fault::NameNotUtf8)],
            ..Judged::default()
        };
    };
    let reject = |line, fault| Judged {
        covers: Some(skill.to_owned()),
        problems: vec![problem(line, fault)],
        cases: Vec::new(),
    };
    let fixture = match source.parsed {
        Err((line, fault)) => return reject(line, fault),
        Ok(fixture) => fixture,
    };
    if let Some(fault) = roster.skill_fault(skill) {
        return reject(0, fault);
    }
    let scorer = &roster.scorer;
    let mut judged = Judged {
        covers: Some(skill.to_owned()),
        problems: counts(&fixture)
            .map(|(line, fault)| problem(line, fault))
            .collect(),
        cases: Vec::with_capacity(
            fixture
                .positives
                .len()
                .saturating_add(fixture.near_misses.len()),
        ),
    };
    for prompt in fixture.positives {
        judged.cases.push(positive(file, skill, prompt, scorer));
    }
    // Every owner is checked, even one that lists no prompts (the reader forbids that, but
    // the judge does not rely on it): such an owner is reported at its key.
    for owner in &fixture.owners {
        let has_prompts = fixture.near_misses.iter().any(|n| n.owner == owner.name);
        if let Some(fault) = roster
            .owner_fault(&owner.name, skill)
            .filter(|_| !has_prompts)
        {
            judged.problems.push(problem(owner.line, fault));
        }
    }
    for near in fixture.near_misses {
        match roster.owner_fault(&near.owner, skill) {
            Some(fault) => judged.problems.push(problem(near.prompt.line, fault)),
            None => judged.cases.push(near_miss(file, skill, near, scorer)),
        }
    }
    judged
}

/// The count faults of `fixture`, each at its list's key (line 1 when the key is absent).
fn counts(fixture: &Fixture) -> impl Iterator<Item = (usize, Fault)> {
    [
        (
            List::Positives,
            fixture.positives.len(),
            fixture.positives_line,
        ),
        (
            List::NearMisses,
            fixture.near_misses.len(),
            fixture.near_misses_line,
        ),
    ]
    .into_iter()
    .filter(|(_, count, _)| !(MIN_PROMPTS..=MAX_PROMPTS).contains(count))
    .map(|(list, count, line)| (line.max(1), Fault::Count { list, count }))
}

/// A positive passes when `skill` is the unique top scorer. Its rival is the best other skill
/// (the first in name order on a tie).
fn positive(file: &str, skill: &str, prompt: fixture::Prompt, scorer: &Scorer<'_>) -> Case {
    let ranking = scorer.score(&prompt.text);
    let top = ranking.top();
    let rival = ranking.iter().filter(|(name, _)| *name != skill).fold(
        None,
        |best: Option<(&str, usize)>, (name, score)| match best {
            Some((_, high)) if high >= score => best,
            _ => Some((name, score)),
        },
    );
    Case {
        file: file.to_owned(),
        line: prompt.line,
        kind: CaseKind::Positive,
        prompt: prompt.text,
        expected: skill.to_owned(),
        passed: matches!(&top, Top::Skill(name) if name == skill),
        top,
        expected_score: ranking.of(skill),
        rival: rival.map(|(name, score)| Rival {
            skill: name.to_owned(),
            score,
        }),
    }
}

/// A near miss passes when its owner scores strictly higher than `skill`, the skill it mimics.
fn near_miss(file: &str, skill: &str, near: fixture::NearMiss, scorer: &Scorer<'_>) -> Case {
    let ranking = scorer.score(&near.prompt.text);
    let owner_score = ranking.of(&near.owner);
    let mimic_score = ranking.of(skill);
    Case {
        file: file.to_owned(),
        line: near.prompt.line,
        kind: CaseKind::NearMiss {
            mimics: skill.to_owned(),
        },
        prompt: near.prompt.text,
        expected: near.owner,
        top: ranking.top(),
        expected_score: owner_score,
        rival: Some(Rival {
            skill: skill.to_owned(),
            score: mimic_score,
        }),
        passed: owner_score > mimic_score,
    }
}
