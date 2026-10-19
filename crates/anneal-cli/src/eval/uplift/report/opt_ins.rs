//! What a run's case files ask for that needs the operator's opt-in, as the envelope lists
//! it: gated tools (`--allow-gated-tools`) and fixtures seeded by scaffold scripts
//! (`--allow-scaffolds`).

use std::collections::BTreeSet;

use serde::Serialize;

use crate::envelope::Finding;
use crate::eval::uplift::case::Tool;
use crate::eval::uplift::suites::Case;

/// A case whose file names gated tools, which a run grants it with `--allow-tools`.
#[derive(Debug, Serialize)]
pub(super) struct Granted<'a> {
    /// The case file, relative to the suites directory.
    case: &'a str,
    /// The gated tools it names, in [`Tool`] order.
    tools: Vec<Tool>,
}

/// Each case of `cases` that names gated tools, with them.
pub(super) fn granted(cases: &[Case]) -> Vec<Granted<'_>> {
    cases
        .iter()
        .filter_map(|case| {
            let tools: BTreeSet<Tool> = case
                .body
                .allowed_tools
                .iter()
                .copied()
                .filter(|tool| tool.is_gated())
                .collect();
            (!tools.is_empty()).then(|| Granted {
                case: &case.file,
                tools: tools.into_iter().collect(),
            })
        })
        .collect()
}

/// The finding naming the gated tools `granted` cases get from their files alone, if any.
pub(super) fn granted_finding(granted: &[Granted<'_>]) -> Option<Finding> {
    if granted.is_empty() {
        return None;
    }
    // In `Tool` order, as `--allow-tools` lists them.
    let tools: Vec<&str> = granted
        .iter()
        .flat_map(|case| case.tools.iter().copied())
        .collect::<BTreeSet<Tool>>()
        .into_iter()
        .map(Tool::name)
        .collect();
    let files: Vec<&str> = granted.iter().map(|case| case.case).collect();
    let count = granted.len();
    let noun = if count == 1 { "case" } else { "cases" };
    let its = if count == 1 {
        "its case file"
    } else {
        "their case files"
    };
    Some(Finding::new(
        "eval/uplift/granted-tools",
        format!(
            "a run grants {} to {count} {noun} from {its} alone ({}); review those cases \
             before running them",
            tools.join(", "),
            files.join(", ")
        ),
    ))
}

/// A case seeded from a fixture, whose scaffold script a run runs with `--allow-scaffolds`.
#[derive(Debug, Serialize)]
pub(super) struct Scaffolded<'a> {
    /// The case file, relative to the suites directory.
    case: &'a str,
    /// The fixture's name, under the suites directory's `fixtures/`.
    fixture: &'a str,
}

/// Each case of `cases` seeded from a fixture, with its fixture.
pub(super) fn scaffolded(cases: &[Case]) -> Vec<Scaffolded<'_>> {
    cases
        .iter()
        .filter_map(|case| {
            case.fixture.as_ref().map(|fixture| Scaffolded {
                case: &case.file,
                fixture: fixture.name.as_str(),
            })
        })
        .collect()
}

/// The finding naming the cases a run seeds with scaffold scripts, if any.
pub(super) fn scaffolded_finding(scaffolded: &[Scaffolded<'_>]) -> Option<Finding> {
    if scaffolded.is_empty() {
        return None;
    }
    let files: Vec<&str> = scaffolded.iter().map(|case| case.case).collect();
    let count = scaffolded.len();
    let (noun, fixtures, scripts) = if count == 1 {
        ("case", "a fixture", "a scaffold script that runs")
    } else {
        ("cases", "fixtures", "scaffold scripts that run")
    };
    Some(Finding::new(
        "eval/uplift/scaffolded",
        format!(
            "a run seeds {count} {noun} from {fixtures} with {scripts} outside the eval sandbox \
             with your rights ({}); review those cases and fixtures before running them with \
             --allow-scaffolds",
            files.join(", ")
        ),
    ))
}
