use super::{Blockers, Mismatches, compare};
use crate::dag::ticket::{Status, Ticket, TicketId};
use crate::lint::plan::Task;

fn task(id: &str, blocked_by: &[&str]) -> Task {
    Task {
        id: id.to_owned(),
        title: format!("task {id}"),
        line: 1,
        files: vec!["src/a.rs".to_owned()],
        test: Some("cargo test".to_owned()),
        blocked_by: blocked_by.iter().map(|id| (*id).to_owned()).collect(),
    }
}

fn ticket(id: &str, blocked_by: &[&str]) -> Ticket {
    Ticket {
        id: TicketId::new(id),
        title: format!("ticket {id}"),
        body: String::new(),
        blocked_by: blocked_by.iter().copied().map(TicketId::new).collect(),
        files: Vec::new(),
        status: Status::Open,
    }
}

#[test]
fn a_plan_and_tickets_that_agree_have_no_mismatch() {
    let tasks = [task("T1", &[]), task("T2", &["T1"])];
    let tickets = [ticket("T2", &["T1"]), ticket("T1", &[])];

    let mismatches = compare(&tasks, &tickets);

    assert_eq!(mismatches, Mismatches::default(), "titles are not compared");
    assert!(mismatches.is_empty(), "agree");
    assert!(mismatches.findings().is_empty(), "no finding");
}

#[test]
fn a_task_with_no_ticket_is_a_mismatch() {
    let tasks = [task("T1", &[]), task("T2", &["T1"])];
    let tickets = [ticket("T1", &[])];

    let mismatches = compare(&tasks, &tickets);

    assert_eq!(mismatches.no_ticket, ["T2"], "T2 has no ticket");
    let findings = mismatches.findings();
    assert_eq!(findings.len(), 1, "one finding");
    assert_eq!(findings[0].key, "dag/plan/no-ticket:T2", "key");
    assert_eq!(
        findings[0].message, "plan task T2 has no ticket: add a ticket with id T2",
        "message"
    );
}

#[test]
fn a_ticket_with_no_task_is_a_mismatch() {
    let tasks = [task("T1", &[])];
    let tickets = [ticket("T1", &[]), ticket("T9", &["T1"])];

    let mismatches = compare(&tasks, &tickets);

    assert_eq!(mismatches.no_task, [&TicketId::new("T9")], "T9 has no task");
    let findings = mismatches.findings();
    assert_eq!(findings.len(), 1, "one finding");
    assert_eq!(findings[0].key, "dag/plan/no-task:T9", "key");
    assert_eq!(
        findings[0].message,
        "ticket T9 is no task of the plan: add the task to the plan or remove the ticket",
        "message"
    );
}

#[test]
fn differing_blockers_are_a_mismatch_naming_both_lists() {
    let tasks = [task("T1", &[]), task("T2", &[]), task("T3", &["T1", "T2"])];
    let tickets = [ticket("T1", &[]), ticket("T2", &[]), ticket("T3", &["T1"])];

    let mismatches = compare(&tasks, &tickets);

    assert_eq!(
        mismatches.blockers,
        [Blockers {
            id: "T3",
            plan: vec!["T1", "T2"],
            ticket: vec!["T1"],
        }],
        "T3's ticket dropped T2"
    );
    let findings = mismatches.findings();
    assert_eq!(findings.len(), 1, "one finding");
    assert_eq!(findings[0].key, "dag/plan/blockers:T3", "key");
    assert_eq!(
        findings[0].message,
        "ticket T3 is blocked by [T1], but the plan's task is blocked by [T1, T2]",
        "message"
    );
}

#[test]
fn blocker_order_and_repeats_do_not_matter() {
    let tasks = [task("T1", &[]), task("T2", &[]), task("T3", &["T2", "T1"])];
    let tickets = [
        ticket("T1", &[]),
        ticket("T2", &[]),
        ticket("T3", &["T1", "T2", "T1"]),
    ];

    let mismatches = compare(&tasks, &tickets);

    assert!(mismatches.is_empty(), "same set: {mismatches:?}");
}

#[test]
fn a_ticket_blocker_outside_the_plan_is_not_compared() {
    let tasks = [task("T1", &[]), task("T2", &["T1"])];
    let tickets = [ticket("T1", &["REL-5"]), ticket("T2", &["T1", "REL-5"])];

    let mismatches = compare(&tasks, &tickets);

    assert!(
        mismatches.is_empty(),
        "an outside blocker belongs in the tracker only: {mismatches:?}"
    );
}

#[test]
fn a_dropped_blocker_is_found_beside_an_outside_one() {
    let tasks = [task("T1", &[]), task("T2", &["T1"])];
    let tickets = [ticket("T1", &[]), ticket("T2", &["REL-5"])];

    let mismatches = compare(&tasks, &tickets);

    assert_eq!(
        mismatches.blockers,
        [Blockers {
            id: "T2",
            plan: vec!["T1"],
            ticket: vec![],
        }],
        "only the plan's tasks are compared"
    );
}

#[test]
fn every_kind_is_listed_in_id_order_and_findings_follow_kind_then_id() {
    let tasks = [
        task("10", &[]),
        task("2", &[]),
        task("3", &["2"]),
        task("b", &["2"]),
    ];
    let tickets = [
        ticket("3", &[]),
        ticket("a", &[]),
        ticket("2", &[]),
        ticket("b", &[]),
        ticket("1", &[]),
    ];

    let mismatches = compare(&tasks, &tickets);

    assert_eq!(mismatches.no_ticket, ["10"], "no ticket");
    assert_eq!(
        mismatches.no_task,
        [&TicketId::new("1"), &TicketId::new("a")],
        "numbers first"
    );
    let ids: Vec<&str> = mismatches.blockers.iter().map(|b| b.id).collect();
    assert_eq!(ids, ["3", "b"], "blockers in id order");
    let keys: Vec<String> = mismatches.findings().into_iter().map(|f| f.key).collect();
    assert_eq!(
        keys,
        [
            "dag/plan/no-ticket:10",
            "dag/plan/no-task:1",
            "dag/plan/no-task:a",
            "dag/plan/blockers:3",
            "dag/plan/blockers:b",
        ],
        "kind, then id"
    );
}

#[test]
fn a_ticket_blocking_a_task_through_outside_tickets_is_outside_not_no_task() {
    let tasks = [task("T1", &[]), task("T2", &["T1"])];
    let tickets = [
        ticket("T1", &["REL-5"]),
        ticket("T2", &["T1"]),
        ticket("REL-5", &["REL-4"]),
        ticket("REL-4", &[]),
        ticket("T9", &["REL-4"]),
    ];

    let mismatches = compare(&tasks, &tickets);

    assert_eq!(
        mismatches.no_task,
        [&TicketId::new("T9")],
        "REL-5 blocks T1 and REL-4 blocks REL-5, so both are outside blockers; T9 blocks no task"
    );
    assert!(mismatches.no_ticket.is_empty(), "every task has a ticket");
    assert!(
        mismatches.blockers.is_empty(),
        "outside blockers are not compared"
    );
}

#[test]
fn a_plan_blocker_outside_the_plan_is_not_compared() {
    let tasks = [task("T1", &[]), task("T2", &["T1", "REL-5"])];
    let tickets = [ticket("T1", &[]), ticket("T2", &["T1"])];

    let mismatches = compare(&tasks, &tickets);

    assert!(
        mismatches.is_empty(),
        "only the plan's tasks are compared, on both sides: {mismatches:?}"
    );
}
