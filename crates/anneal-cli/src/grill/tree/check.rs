//! The checks a parsed design tree file must pass, apart from cycles (`cycle`).
//!
//! Each [`TreeProblem`] owns the ids it names (hence the clones): problems are returned after
//! the parsed file is dropped.

use std::collections::BTreeSet;

use super::{MAX_DEPENDENCIES, MAX_ID_LEN, MAX_NODES, MAX_TURNS, Node, TreeFile, TreeProblem};

/// Every problem in `file`, whose nodes are sorted by id and whose format version
/// `DesignTree::parse` has checked: the goal, the bounds, then each turn, then each node with
/// its dependencies. Past a bound nothing else is checked.
pub(super) fn problems(file: &TreeFile) -> Vec<TreeProblem> {
    let mut problems = Vec::new();
    if file.goal.trim().is_empty() {
        problems.push(TreeProblem::EmptyGoal);
    }
    if file.turns.len() > MAX_TURNS {
        problems.push(TreeProblem::TooManyTurns {
            count: file.turns.len(),
        });
    }
    if file.nodes.len() > MAX_NODES {
        problems.push(TreeProblem::TooManyNodes {
            count: file.nodes.len(),
        });
    }
    if file.turns.len() > MAX_TURNS || file.nodes.len() > MAX_NODES {
        return problems;
    }
    for (turn, value) in (1..).zip(file.turns.iter().map(|turn| turn.confidence)) {
        if !in_unit_range(value) {
            problems.push(TreeProblem::TurnConfidence { turn, value });
        }
    }
    // Sorted by id, so the nodes sharing an id are adjacent: the first is checked, the second
    // reports the duplicate, any further one is skipped.
    let mut previous: Option<&str> = None;
    let mut duplicate_reported = false;
    for node in &file.nodes {
        if previous == Some(node.id.as_str()) {
            if !duplicate_reported {
                problems.push(TreeProblem::DuplicateId {
                    id: node.id.clone(),
                });
                duplicate_reported = true;
            }
            continue;
        }
        previous = Some(&node.id);
        duplicate_reported = false;
        node_problems(node, &file.nodes, &mut problems);
    }
    problems
}

/// The problems of one node and its dependencies, appended to `problems`. `nodes` is sorted by
/// id.
fn node_problems(node: &Node, nodes: &[Node], problems: &mut Vec<TreeProblem>) {
    // Each problem owns the ids it names: it outlives the parsed file.
    let id = || node.id.clone();
    if !is_valid_id(&node.id) {
        problems.push(TreeProblem::BadId { id: id() });
    }
    if node.question.trim().is_empty() {
        problems.push(TreeProblem::EmptyQuestion { id: id() });
    }
    if node.answer.trim().is_empty() {
        problems.push(TreeProblem::EmptyAnswer { id: id() });
    }
    if !in_unit_range(node.confidence) {
        problems.push(TreeProblem::NodeConfidence {
            id: id(),
            value: node.confidence,
        });
    }
    if !in_unit_range(node.is_decision) {
        problems.push(TreeProblem::IsDecision {
            id: id(),
            value: node.is_decision,
        });
    }
    // Past the bound, one problem for the node rather than one per dependency (H-13).
    if node.depends_on.len() > MAX_DEPENDENCIES {
        problems.push(TreeProblem::TooManyDependencies {
            id: id(),
            count: node.depends_on.len(),
        });
        return;
    }
    let mut seen = BTreeSet::new();
    for dependency in &node.depends_on {
        let known = nodes
            .binary_search_by(|other| other.id.as_str().cmp(dependency))
            .is_ok();
        if !known {
            problems.push(TreeProblem::DanglingDependency {
                id: id(),
                dependency: dependency.clone(),
            });
        } else if !seen.insert(dependency.as_str()) {
            problems.push(TreeProblem::DuplicateDependency {
                id: id(),
                dependency: dependency.clone(),
            });
        }
    }
}

/// `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`, the schema's id pattern.
fn is_valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && id.len() <= MAX_ID_LEN
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn in_unit_range(value: f64) -> bool {
    (0.0..=1.0).contains(&value)
}
