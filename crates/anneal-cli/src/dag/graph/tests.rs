use super::{Dangling, Graph, GraphError};
use crate::dag::ticket::{Status, Ticket, TicketId};
use crate::exit_codes;

fn ticket(id: &str, blocked_by: &[&str], status: Status) -> Ticket {
    Ticket {
        id: TicketId::new(id),
        title: format!("ticket {id}"),
        body: String::new(),
        blocked_by: blocked_by.iter().map(|b| TicketId::new(*b)).collect(),
        files: Vec::new(),
        status,
    }
}

fn open(id: &str, blocked_by: &[&str]) -> Ticket {
    ticket(id, blocked_by, Status::Open)
}

fn graph(tickets: Vec<Ticket>) -> Graph {
    Graph::new(tickets).expect("unique ids")
}

fn ids<'a>(tickets: impl IntoIterator<Item = &'a Ticket>) -> Vec<&'a str> {
    tickets.into_iter().map(|t| t.id.as_str()).collect()
}

fn cycle_of(graph: &Graph) -> Option<Vec<&str>> {
    graph
        .check()
        .cycle
        .map(|cycle| cycle.iter().map(|id| id.as_str()).collect())
}

#[test]
fn an_empty_graph_checks_ok_with_an_empty_frontier() {
    let graph = graph(Vec::new());

    let check = graph.check();

    assert_eq!(check.code(), exit_codes::OK, "nothing to fault");
    assert!(check.dangling.is_empty(), "no refs");
    assert_eq!(check.cycle, None, "no cycle");
    assert!(graph.frontier().next().is_none(), "nothing ready");
}

#[test]
fn an_acyclic_graph_checks_ok() {
    let graph = graph(vec![
        open("1", &[]),
        open("2", &["1"]),
        open("3", &["1", "2"]),
    ]);

    let check = graph.check();

    assert_eq!(check.code(), exit_codes::OK, "acyclic and complete");
    assert_eq!(graph.edges(), 3, "three blocking edges");
}

#[test]
fn a_cycle_is_named_in_order_from_its_smallest_id() {
    // 3 is blocked by 4, 4 by 2, 2 by 3: from the smallest id, 2 -> 3 -> 4 -> back to 2.
    let graph = graph(vec![
        open("1", &[]),
        open("3", &["4"]),
        open("4", &["2"]),
        open("2", &["3", "1"]),
    ]);

    let check = graph.check();

    assert_eq!(check.code(), exit_codes::DAG_CYCLE, "a cycle exits 1");
    assert_eq!(
        cycle_of(&graph),
        Some(vec!["2", "3", "4"]),
        "each id is blocked by the next, the last by the first"
    );
}

#[test]
fn a_ticket_blocked_by_itself_is_a_cycle_of_one() {
    let graph = graph(vec![open("a", &["a"])]);

    assert_eq!(cycle_of(&graph), Some(vec!["a"]), "self-loop");
    assert_eq!(graph.check().code(), exit_codes::DAG_CYCLE, "exit 1");
}

#[test]
fn each_dangling_reference_is_named_with_its_ticket() {
    let graph = graph(vec![open("2", &["9", "1", "8"]), open("1", &["7"])]);

    let check = graph.check();

    assert_eq!(check.code(), exit_codes::DAG_DANGLING, "exit 2");
    let named: Vec<(&str, &str)> = check
        .dangling
        .iter()
        .map(|Dangling { ticket, missing }| (ticket.as_str(), missing.as_str()))
        .collect();
    assert_eq!(
        named,
        [("1", "7"), ("2", "8"), ("2", "9")],
        "by ticket, then by missing id"
    );
}

#[test]
fn dangling_takes_precedence_over_a_cycle_and_both_are_reported() {
    let graph = graph(vec![open("1", &["2"]), open("2", &["1", "404"])]);

    let check = graph.check();

    assert_eq!(check.code(), exit_codes::DAG_DANGLING, "dangling wins");
    assert_eq!(check.dangling.len(), 1, "the missing ref");
    assert!(check.cycle.is_some(), "the cycle is still reported");
}

#[test]
fn duplicate_ids_are_an_error_naming_the_id() {
    let result = Graph::new(vec![open("1", &[]), open("1", &[])]);

    assert!(
        matches!(&result, Err(GraphError::DuplicateId { id }) if id.as_str() == "1"),
        "got {result:?}"
    );
}

#[test]
fn the_frontier_is_open_tickets_whose_blockers_are_all_done_in_id_order() {
    let graph = graph(vec![
        open("10", &[]),
        ticket("1", &[], Status::Done),
        open("3", &["1"]),
        open("2", &["1", "10"]),
        open("b", &[]),
        open("a", &["1"]),
        ticket("4", &[], Status::Done),
    ]);

    assert_eq!(
        ids(graph.frontier()),
        ["3", "10", "a", "b"],
        "2 waits for 10; done tickets are never ready; numbers before text"
    );
}

#[test]
fn a_long_chain_is_checked_without_recursion() {
    let length = 100_000_usize;
    let tickets = (0..length)
        .map(|n| {
            let blocked_by: Vec<String> = n
                .checked_sub(1)
                .map(|m| m.to_string())
                .into_iter()
                .collect();
            let refs: Vec<&str> = blocked_by.iter().map(String::as_str).collect();
            open(&n.to_string(), &refs)
        })
        .collect();
    let graph = graph(tickets);

    assert_eq!(graph.check().code(), exit_codes::OK, "a deep chain is fine");
    assert_eq!(ids(graph.frontier()), ["0"], "only the head is ready");
}

#[test]
fn external_tickets_resolve_references_but_are_leaves_never_counted_or_ready() {
    let graph = Graph::with_external(
        vec![
            open("1", &["e-done"]),
            open("2", &["e-open"]),
            open("3", &[]),
        ],
        vec![
            ticket("e-done", &["nowhere"], Status::Done),
            ticket("e-open", &["2"], Status::Open),
        ],
    )
    .expect("unique ids");

    assert_eq!(
        graph.check().code(),
        exit_codes::OK,
        "no dangling ref, no cycle through 2"
    );
    assert_eq!(
        ids(graph.tickets()),
        ["1", "2", "3"],
        "external tickets are not counted"
    );
    assert_eq!(
        ids(graph.frontier()),
        ["1", "3"],
        "2 waits for the open external ticket"
    );
    let blockers: Vec<(&str, Vec<&str>)> = graph
        .external_blockers()
        .map(|b| {
            (
                b.ticket.id.as_str(),
                b.blocks.iter().map(|id| id.as_str()).collect(),
            )
        })
        .collect();
    assert_eq!(
        blockers,
        [("e-open", vec!["2"])],
        "the open external blocker and what it holds"
    );
}

#[test]
fn an_external_ticket_sharing_an_id_with_one_in_scope_is_a_duplicate() {
    let result = Graph::with_external(vec![open("1", &[])], vec![open("1", &[])]);

    assert!(
        matches!(&result, Err(GraphError::DuplicateId { id }) if id.as_str() == "1"),
        "got {result:?}"
    );
}
