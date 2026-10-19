//! The opt-ins a case file alone cannot grant: a run whose selected cases name `Write`, `Edit`
//! or `Bash` is refused unless `--allow-gated-tools` is given, and one whose selected cases
//! name a fixture, seeded by a scaffold script that runs outside the eval sandbox with the
//! user's rights, unless `--allow-scaffolds` is given.

use std::fmt;

use super::suites::Case;

/// How many case files a refusal names before it only counts the rest, so its one `ERROR`
/// line stays short whatever the selection.
const MAX_NAMED: usize = 3;

/// The selected cases that need an opt-in (a gated tool or a fixture), as a refusal names
/// them: the first few files, in selection order, and how many there are in all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatedCases {
    /// The first [`MAX_NAMED`] case files, relative to the suites directory.
    named: Vec<String>,
    /// How many selected cases need the opt-in.
    count: usize,
}

impl fmt::Display for GatedCases {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let noun = if self.count == 1 { "case" } else { "cases" };
        write!(
            f,
            "{} selected {noun} ({}",
            self.count,
            self.named.join(", ")
        )?;
        match self.count.saturating_sub(self.named.len()) {
            0 => f.write_str(")"),
            more => write!(f, " and {more} more)"),
        }
    }
}

/// The cases of `cases` that name a gated tool, or `None` when none does and the run needs
/// no opt-in.
pub(crate) fn refusal(cases: &[Case]) -> Option<GatedCases> {
    refusal_by(cases, |case| {
        case.body.allowed_tools.iter().any(|tool| tool.is_gated())
    })
}

/// The cases of `cases` seeded from a fixture, whose scaffold scripts run outside the eval
/// sandbox, or `None` when none is and the run needs no `--allow-scaffolds`.
pub(crate) fn scaffold_refusal(cases: &[Case]) -> Option<GatedCases> {
    refusal_by(cases, |case| case.fixture.is_some())
}

/// The cases of `cases` for which `gated` holds, or `None` when it holds for none.
fn refusal_by(cases: &[Case], gated: impl Fn(&Case) -> bool) -> Option<GatedCases> {
    let mut gated = cases.iter().filter(|case| gated(case));
    // The error outlives the suites, so it owns the few names it shows.
    let named: Vec<String> = gated
        .by_ref()
        .take(MAX_NAMED)
        .map(|case| case.file.clone())
        .collect();
    if named.is_empty() {
        return None;
    }
    let count = named.len().saturating_add(gated.count());
    Some(GatedCases { named, count })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{GatedCases, refusal, scaffold_refusal};
    use crate::eval::uplift::case::parse;
    use crate::eval::uplift::fixture::Fixture;
    use crate::eval::uplift::suites::{Case, SuiteKind};

    fn case(slug: &str, tools: &[&str]) -> Case {
        let text = json!({
            "behaviour": "outcome",
            "seed": "s",
            "prompt": "p",
            "allowed_tools": tools,
            "graders": [{"type": "regex", "name": "a", "pattern": "x"}]
        })
        .to_string();
        Case {
            skill: "plan".to_owned(),
            suite: SuiteKind::Regression,
            slug: slug.to_owned(),
            file: format!("plan/regression/{slug}.json"),
            body: parse(text.as_bytes()).expect("valid"),
            fixture: None,
        }
    }

    #[test]
    fn cases_seeded_from_a_fixture_need_the_scaffold_opt_in() {
        let seeded = |slug: &str| Case {
            fixture: Some(Fixture::sample(slug)),
            ..case(slug, &["Skill"])
        };
        let cases = [case("a", &["Skill", "Bash"]), seeded("b"), seeded("c")];

        let refused = (scaffold_refusal(&cases), scaffold_refusal(&cases[..1]));

        assert_eq!(
            (refused.0.as_ref().map(ToString::to_string), refused.1),
            (
                Some(
                    "2 selected cases (plan/regression/b.json, plan/regression/c.json)".to_owned()
                ),
                None
            )
        );
    }

    #[test]
    fn a_selection_with_no_gated_tool_needs_no_opt_in() {
        let cases = [case("a", &["Skill", "Read", "Grep", "Glob"])];

        assert_eq!(refusal(&cases), None);
    }

    #[test]
    fn one_gated_case_is_named() {
        let cases = [case("a", &["Skill", "Read"]), case("b", &["Skill", "Bash"])];

        let refused = refusal(&cases);

        assert_eq!(
            refused.as_ref().map(ToString::to_string),
            Some("1 selected case (plan/regression/b.json)".to_owned())
        );
    }

    #[test]
    fn exactly_three_gated_cases_are_all_named_with_nothing_more_counted() {
        let cases = [
            case("a", &["Skill", "Write"]),
            case("b", &["Skill", "Read"]),
            case("c", &["Skill", "Edit"]),
            case("d", &["Skill", "Bash"]),
        ];

        let refused = refusal(&cases);

        assert_eq!(
            refused.as_ref().map(ToString::to_string),
            Some(
                "3 selected cases (plan/regression/a.json, plan/regression/c.json, \
                 plan/regression/d.json)"
                    .to_owned()
            )
        );
    }

    #[test]
    fn past_three_gated_cases_the_rest_are_counted() {
        let cases = [
            case("a", &["Skill", "Write"]),
            case("b", &["Skill", "Edit"]),
            case("c", &["Skill", "Bash"]),
            case("d", &["Skill", "Bash"]),
            case("e", &["Skill", "Read"]),
            case("f", &["Skill", "Edit"]),
        ];

        let refused = refusal(&cases);

        assert_eq!(
            refused,
            Some(GatedCases {
                named: vec![
                    "plan/regression/a.json".to_owned(),
                    "plan/regression/b.json".to_owned(),
                    "plan/regression/c.json".to_owned(),
                ],
                count: 5,
            })
        );
        assert_eq!(
            refused.as_ref().map(ToString::to_string),
            Some(
                "5 selected cases (plan/regression/a.json, plan/regression/b.json, \
                 plan/regression/c.json and 2 more)"
                    .to_owned()
            )
        );
    }
}
