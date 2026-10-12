use anneal_ledger::Store;

use super::{
    Attempt, Next, Outcome, START_ATTEMPTS, count, count_within, envelope_for, start_once,
    start_tries,
};
use crate::brief::ticket::Ticket;
use crate::error::CliError;
use crate::exit_codes;

/// Two handles on one fresh ledger: the `start` under test and another writer.
fn two_writers() -> (tempfile::TempDir, Store, Store) {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("ledger.sqlite3");
    let store = Store::open(&path).expect("opened");
    let other = Store::open(&path).expect("opened again");
    (dir, store, other)
}

#[test]
fn a_start_that_loses_to_the_same_ticket_reports_the_winning_round_and_appends_nothing() {
    let (_dir, store, other) = two_writers();
    let ticket: Ticket = "36".parse().expect("a ticket");
    let seen = count(&store, &ticket).expect("counted");
    let winner = envelope_for(&ticket, 1, None).expect("an envelope");
    other.append(&winner).expect("the other start won");

    let attempt = start_once(&store, &ticket, seen, None).expect("judged");

    assert_eq!(
        attempt,
        Attempt::Done(Outcome::Joined { rounds: 1 }),
        "round 1 started once, by the other writer"
    );
    assert_eq!(
        count(&store, &ticket).expect("counted").rounds,
        1,
        "no round 2 recorded behind it"
    );
}

#[test]
fn a_start_that_loses_to_another_ticket_is_tried_again() {
    let (_dir, store, other) = two_writers();
    let ticket: Ticket = "36".parse().expect("a ticket");
    let neighbour: Ticket = "37".parse().expect("a ticket");
    let seen = count(&store, &ticket).expect("counted");
    let theirs = envelope_for(&neighbour, 1, None).expect("an envelope");
    other.append(&theirs).expect("another ticket's round");

    let attempt = start_once(&store, &ticket, seen, None).expect("judged");

    assert_eq!(attempt, Attempt::Retry, "the read is stale, not lost");
    assert_eq!(
        count(&store, &ticket).expect("counted").rounds,
        0,
        "nothing appended for 36 yet"
    );
}

#[test]
fn a_start_that_loses_every_try_to_other_tickets_is_contended() {
    let (_dir, store, other) = two_writers();
    let ticket: Ticket = "36".parse().expect("a ticket");
    let neighbour: Ticket = "37".parse().expect("a ticket");
    let mut tries = 0_usize;

    let result = start_tries(|| {
        tries += 1;
        let seen = count(&store, &ticket)?;
        let round = u32::try_from(tries).expect("a few tries");
        other
            .append(&envelope_for(&neighbour, round, None)?)
            .expect("another ticket's round in between");
        start_once(&store, &ticket, seen, None)
    });

    assert!(
        matches!(result, Err(CliError::RoundContended { attempts: 4 })),
        "four lost tries: {result:?}"
    );
    assert_eq!(tries, START_ATTEMPTS, "tried exactly START_ATTEMPTS times");
    assert_eq!(
        count(&store, &ticket).expect("counted").rounds,
        0,
        "nothing appended for 36"
    );
}

#[test]
fn a_start_that_wins_a_later_try_stops_trying() {
    let (_dir, store, other) = two_writers();
    let ticket: Ticket = "36".parse().expect("a ticket");
    let neighbour: Ticket = "37".parse().expect("a ticket");
    let mut tries = 0_usize;

    let result = start_tries(|| {
        tries += 1;
        let seen = count(&store, &ticket)?;
        if tries == 1 {
            other
                .append(&envelope_for(&neighbour, 1, None)?)
                .expect("another ticket's round in between");
        }
        start_once(&store, &ticket, seen, None)
    });

    assert!(
        matches!(result, Ok(Outcome::Started { round: 1, .. })),
        "the second try records round 1: {result:?}"
    );
    assert_eq!(tries, 2, "no third try");
}

#[test]
fn rounds_are_counted_up_to_the_cap_and_more_round_events_are_an_internal_error() {
    let (_dir, store, _other) = two_writers();
    let ticket: Ticket = "36".parse().expect("a ticket");
    let neighbour: Ticket = "37".parse().expect("a ticket");
    store
        .append(&envelope_for(&ticket, 1, None).expect("an envelope"))
        .expect("round 1 of 36");
    store
        .append(&envelope_for(&neighbour, 1, None).expect("an envelope"))
        .expect("round 1 of 37");

    let at_cap = count_within(&store, &ticket, 2);
    let past_cap = count_within(&store, &ticket, 1);

    assert_eq!(
        at_cap.expect("two events fit a cap of two").rounds,
        1,
        "counted whole"
    );
    let error = past_cap.expect_err("two events do not fit a cap of one");
    assert!(
        matches!(error, CliError::TooManyRounds { cap: 1 }),
        "never a count of a cut history: {error:?}"
    );
    assert_eq!(error.code(), exit_codes::INTERNAL, "exit 70");
}

#[test]
fn running_out_of_tries_is_an_internal_error_not_a_usage_error() {
    assert_eq!(
        CliError::RoundContended { attempts: 4 }.code(),
        exit_codes::INTERNAL,
        "the ledger's contention, not the caller's mistake"
    );
}

#[test]
fn rounds_one_to_three_use_the_same_agent_four_and_five_a_fresh_one_then_park() {
    let plans: Vec<Next> = (0..=7).map(Next::for_round).collect();

    assert_eq!(
        plans,
        [
            Next::Park,
            Next::SameAgent,
            Next::SameAgent,
            Next::SameAgent,
            Next::FreshAgent,
            Next::FreshAgent,
            Next::Park,
            Next::Park,
        ],
        "round 0 does not exist; after round 5 nothing runs"
    );
}

#[test]
fn the_next_round_follows_the_rounds_already_started() {
    assert_eq!(
        [0, 3, 5, u32::MAX].map(Next::after),
        [Next::SameAgent, Next::FreshAgent, Next::Park, Next::Park],
        "the round after the count, saturating"
    );
}
