use serde_json::{Value, json};

use super::{ConfidenceSummary, DesignTree, Status, TreeProblem};

/// A node as JSON: a fact (`is_decision` 0.1) with a recommended answer and confidence 0.5.
fn node(id: &str, depends_on: &[&str], status: &str) -> Value {
    json!({
        "id": id,
        "question": format!("question {id}?"),
        "depends_on": depends_on,
        "status": status,
        "answer": format!("recommended answer to {id}"),
        "is_decision": 0.1,
        "confidence": 0.5,
    })
}

#[test]
fn a_node_is_asked_from_an_is_decision_of_0_5_and_looked_up_below() {
    let mut nodes = Vec::new();
    for (id, is_decision) in [("a", 0.49), ("b", 0.5), ("c", 1.0), ("d", 0.0)] {
        let mut open = node(id, &[], "open");
        open["is_decision"] = json!(is_decision);
        nodes.push(open);
    }
    let tree = parse(&tree(&nodes, &[])).expect("a valid tree");

    let asked: Vec<(&str, bool)> = tree
        .frontier()
        .map(|node| (node.id.as_str(), node.is_asked()))
        .collect();

    assert_eq!(
        asked,
        [("a", false), ("b", true), ("c", true), ("d", false)],
        "0.5 is the line between asking the human and looking it up"
    );
}

/// A version-1 tree with `nodes` and per-turn `turns` confidences.
fn tree(nodes: &[Value], turns: &[f64]) -> Vec<u8> {
    let turns: Vec<Value> = turns.iter().map(|c| json!({"confidence": c})).collect();
    serde_json::to_vec(&json!({
        "version": 1,
        "goal": "export orders as CSV",
        "turns": turns,
        "nodes": nodes,
    }))
    .expect("a json! value serialises")
}

fn parse(bytes: &[u8]) -> Result<DesignTree, Vec<TreeProblem>> {
    DesignTree::parse(bytes)
}

fn frontier_ids(tree: &DesignTree) -> Vec<&str> {
    tree.frontier().map(|node| node.id.as_str()).collect()
}

#[test]
fn the_frontier_is_the_open_nodes_whose_dependencies_are_all_answered_sorted_by_id() {
    let bytes = tree(
        &[
            node("scope", &[], "answered"),
            node("format", &["scope"], "open"),
            node("audience", &[], "open"),
            node("columns", &["format"], "open"),
        ],
        &[],
    );

    let tree = parse(&bytes).expect("a valid tree");

    assert_eq!(
        frontier_ids(&tree),
        ["audience", "format"],
        "open, every dependency answered, ordered by id"
    );
}

#[test]
fn a_node_waiting_on_an_open_or_deferred_dependency_is_not_on_the_frontier() {
    let bytes = tree(
        &[
            node("a-open", &[], "open"),
            node("b-deferred", &[], "deferred"),
            node("c-after-open", &["a-open"], "open"),
            node("d-after-deferred", &["b-deferred"], "open"),
            node("e-answered", &[], "answered"),
        ],
        &[],
    );

    let tree = parse(&bytes).expect("a valid tree");

    assert_eq!(
        frontier_ids(&tree),
        ["a-open"],
        "only answered dependencies unblock; answered and deferred nodes are never on it"
    );
}

#[test]
fn a_depends_on_cycle_is_rejected_and_named() {
    let bytes = tree(
        &[
            node("a", &["c"], "open"),
            node("b", &["a"], "open"),
            node("c", &["b"], "open"),
            node("d", &[], "open"),
        ],
        &[],
    );

    assert_eq!(
        parse(&bytes),
        Err(vec![TreeProblem::Cycle {
            path: vec![
                "a".to_owned(),
                "c".to_owned(),
                "b".to_owned(),
                "a".to_owned()
            ],
        }]),
        "the cycle is walked from its smallest id along depends_on"
    );
}

#[test]
fn a_node_depending_on_itself_is_a_cycle() {
    let bytes = tree(&[node("a", &["a"], "open")], &[]);

    assert_eq!(
        parse(&bytes),
        Err(vec![TreeProblem::Cycle {
            path: vec!["a".to_owned(), "a".to_owned()],
        }]),
        "a self-dependency is a cycle of one"
    );
}

#[test]
fn a_node_downstream_of_a_cycle_does_not_hide_it() {
    let bytes = tree(
        &[
            node("a", &["z"], "open"),
            node("y", &["z"], "open"),
            node("z", &["y"], "open"),
        ],
        &[],
    );

    assert_eq!(
        parse(&bytes),
        Err(vec![TreeProblem::Cycle {
            path: vec!["y".to_owned(), "z".to_owned(), "y".to_owned()],
        }]),
        "only the cycle is named, from its smallest id, not the node waiting on it"
    );
}

#[test]
fn a_dependency_naming_no_node_is_dangling() {
    let bytes = tree(&[node("a", &["ghost"], "open")], &[]);

    assert_eq!(
        parse(&bytes),
        Err(vec![TreeProblem::DanglingDependency {
            id: "a".to_owned(),
            dependency: "ghost".to_owned(),
        }]),
        "every dependency names a node"
    );
}

#[test]
fn a_dependency_listed_twice_is_rejected() {
    let bytes = tree(
        &[node("a", &[], "answered"), node("b", &["a", "a"], "open")],
        &[],
    );

    assert_eq!(
        parse(&bytes),
        Err(vec![TreeProblem::DuplicateDependency {
            id: "b".to_owned(),
            dependency: "a".to_owned(),
        }]),
        "depends_on items are unique"
    );
}

#[test]
fn two_nodes_with_one_id_are_rejected_once() {
    let bytes = tree(
        &[
            node("a", &[], "open"),
            node("a", &[], "answered"),
            node("a", &[], "deferred"),
        ],
        &[],
    );

    assert_eq!(
        parse(&bytes),
        Err(vec![TreeProblem::DuplicateId { id: "a".to_owned() }]),
        "ids are unique; a repeated id is reported once"
    );
}

#[test]
fn an_id_outside_the_pattern_is_rejected() {
    let long = "x".repeat(65);
    for id in ["", "-lead", "has space", "a/b", "caf\u{e9}", long.as_str()] {
        let bytes = tree(&[node(id, &[], "open")], &[]);

        assert_eq!(
            parse(&bytes),
            Err(vec![TreeProblem::BadId { id: id.to_owned() }]),
            "{id:?} is not a valid id"
        );
    }
}

#[test]
fn ids_inside_the_pattern_are_accepted() {
    let longest = "x".repeat(64);
    for id in ["a", "Q1", "scope.format_2-b", longest.as_str()] {
        let bytes = tree(&[node(id, &[], "open")], &[]);

        assert!(parse(&bytes).is_ok(), "{id:?} is a valid id");
    }
}

#[test]
fn an_empty_question_answer_or_goal_is_rejected() {
    let mut blank = node("a", &[], "open");
    blank["question"] = json!("  ");
    blank["answer"] = json!("");
    let mut bytes: Value = serde_json::from_slice(&tree(&[blank], &[])).expect("json");
    bytes["goal"] = json!("\t");

    assert_eq!(
        parse(&serde_json::to_vec(&bytes).expect("json")),
        Err(vec![
            TreeProblem::EmptyGoal,
            TreeProblem::EmptyQuestion { id: "a".to_owned() },
            TreeProblem::EmptyAnswer { id: "a".to_owned() },
        ]),
        "every question carries a recommended answer; blank text counts as empty"
    );
}

#[test]
fn a_confidence_outside_0_to_1_is_rejected_for_nodes_and_turns() {
    let mut high = node("a", &[], "open");
    high["confidence"] = json!(1.5);
    let mut low = node("b", &[], "open");
    low["confidence"] = json!(-0.1);
    let bytes = tree(&[high, low], &[0.0, 1.0, 1.01]);

    assert_eq!(
        parse(&bytes),
        Err(vec![
            TreeProblem::TurnConfidence {
                turn: 3,
                value: 1.01
            },
            TreeProblem::NodeConfidence {
                id: "a".to_owned(),
                value: 1.5
            },
            TreeProblem::NodeConfidence {
                id: "b".to_owned(),
                value: -0.1
            },
        ]),
        "0 and 1 are inside; turns count from 1"
    );
}

#[test]
fn an_is_decision_outside_0_to_1_is_rejected() {
    let mut over = node("a", &[], "open");
    over["is_decision"] = json!(2);

    assert_eq!(
        parse(&tree(&[over], &[])),
        Err(vec![TreeProblem::IsDecision {
            id: "a".to_owned(),
            value: 2.0
        }]),
        "is_decision is a probability"
    );
}

#[test]
fn an_unknown_field_or_status_is_not_a_design_tree() {
    let mut extra = node("a", &[], "open");
    extra["priority"] = json!(1);
    let unknown_status = node("a", &[], "closed");
    for bad in [extra, unknown_status] {
        let problems = parse(&tree(&[bad], &[])).expect_err("rejected");

        assert!(
            matches!(problems.as_slice(), [TreeProblem::Json { .. }]),
            "one JSON problem: {problems:?}"
        );
    }
}

#[test]
fn another_version_is_rejected() {
    let mut value: Value = serde_json::from_slice(&tree(&[], &[])).expect("json");
    value["version"] = json!(2);

    assert_eq!(
        parse(&serde_json::to_vec(&value).expect("json")),
        Err(vec![TreeProblem::Version { found: 2 }]),
        "only format version 1 is read"
    );
}

#[test]
fn more_nodes_or_turns_than_the_bounds_are_rejected() {
    let nodes: Vec<Value> = (0..=super::MAX_NODES)
        .map(|n| node(&format!("n{n}"), &[], "open"))
        .collect();
    let turns = vec![0.5; super::MAX_TURNS + 1];

    assert_eq!(
        parse(&tree(&nodes, &turns)),
        Err(vec![
            TreeProblem::TooManyTurns { count: 257 },
            TreeProblem::TooManyNodes { count: 513 },
        ]),
        "512 nodes and 256 turns at most"
    );
}

#[test]
fn nodes_are_listed_by_status_in_id_order() {
    let bytes = tree(
        &[
            node("c", &[], "open"),
            node("b", &[], "deferred"),
            node("a", &["c"], "open"),
            node("d", &[], "answered"),
        ],
        &[],
    );
    let tree = parse(&bytes).expect("a valid tree");

    let ids = |status| -> Vec<&str> {
        tree.with_status(status)
            .map(|node| node.id.as_str())
            .collect()
    };

    assert_eq!(
        (
            ids(Status::Open),
            ids(Status::Answered),
            ids(Status::Deferred)
        ),
        (vec!["a", "c"], vec!["d"], vec!["b"]),
        "each status, sorted by id, blocked or not"
    );
}

fn confidence(turns: &[f64]) -> ConfidenceSummary {
    parse(&tree(&[], turns)).expect("a valid tree").confidence()
}

#[test]
fn no_turns_yet_means_no_confidence_and_no_restate() {
    assert_eq!(
        confidence(&[]),
        ConfidenceSummary {
            turns: 0,
            latest: None,
            best: None,
            turns_without_gain: 0,
            restate: false,
        },
        "nothing recorded"
    );
}

#[test]
fn rising_confidence_below_the_threshold_does_not_restate() {
    assert_eq!(
        confidence(&[0.3, 0.5, 0.79]),
        ConfidenceSummary {
            turns: 3,
            latest: Some(0.79),
            best: Some(0.79),
            turns_without_gain: 0,
            restate: false,
        },
        "every turn gained, and 0.79 is under 0.8"
    );
}

#[test]
fn reaching_0_8_restates() {
    let summary = confidence(&[0.5, 0.8]);

    assert_eq!(
        (summary.latest, summary.restate),
        (Some(0.8), true),
        "0.8 is the threshold"
    );
}

#[test]
fn three_turns_without_gain_restate() {
    assert_eq!(
        confidence(&[0.4, 0.6, 0.6, 0.5, 0.6]),
        ConfidenceSummary {
            turns: 5,
            latest: Some(0.6),
            best: Some(0.6),
            turns_without_gain: 3,
            restate: true,
        },
        "a turn gains only by beating the best before it"
    );
}

#[test]
fn a_gain_resets_the_count_of_turns_without_gain() {
    let summary = confidence(&[0.4, 0.4, 0.4, 0.5]);

    assert_eq!(
        (summary.turns_without_gain, summary.restate),
        (0, false),
        "the last turn beat 0.4"
    );
}

#[test]
fn a_file_over_the_byte_limit_is_rejected_before_parsing() {
    let bytes = vec![b' '; 1_048_577];

    assert_eq!(
        parse(&bytes),
        Err(vec![TreeProblem::TooLarge { limit: 1_048_576 }]),
        "1 MiB at most"
    );
}
