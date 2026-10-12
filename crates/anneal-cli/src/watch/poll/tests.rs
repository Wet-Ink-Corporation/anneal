use std::cell::RefCell;
use std::time::Duration;

use super::{Schedule, Source, watch};
use crate::watch::WatchError;
use crate::watch::fake::FakeClock;
use crate::watch::snapshot::{Blocker, Pending, Snapshot};

/// A source that answers each poll with the next snapshot, repeating the last one.
struct Script(RefCell<Vec<Snapshot>>);

impl Script {
    fn new(mut snapshots: Vec<Snapshot>) -> Self {
        snapshots.reverse();
        Self(RefCell::new(snapshots))
    }
}

impl Source for Script {
    fn poll(&self) -> Result<Snapshot, WatchError> {
        let mut left = self.0.borrow_mut();
        let next = if left.len() > 1 {
            left.pop()
        } else {
            left.last().cloned()
        };
        Ok(next.expect("the script has a snapshot"))
    }
}

fn pending() -> Snapshot {
    Snapshot {
        head: None,
        blockers: Vec::new(),
        pending: vec![Pending::NoChecks],
    }
}

fn clear() -> Snapshot {
    Snapshot::default()
}

fn failing() -> Snapshot {
    Snapshot {
        head: None,
        blockers: vec![Blocker::Check {
            name: "test".to_owned(),
            conclusion: "failure".to_owned(),
            url: None,
        }],
        pending: vec![Pending::NoChecks],
    }
}

const fn secs(secs: u64) -> Duration {
    Duration::from_secs(secs)
}

#[test]
fn a_clear_first_poll_returns_without_sleeping() {
    let clock = FakeClock::default();

    let watched = watch(
        &Script::new(vec![clear()]),
        &clock,
        Schedule::with_timeout(secs(1800)),
    )
    .expect("the fake never fails");

    assert_eq!(watched.polls, 1, "one poll");
    assert!(!watched.timed_out, "settled");
    assert_eq!(*clock.sleeps.borrow(), Vec::<Duration>::new(), "no sleep");
}

#[test]
fn pending_polls_back_off_doubling_to_the_cap_and_never_sleep_past_the_deadline() {
    let clock = FakeClock::default();

    let watched = watch(
        &Script::new(vec![pending()]),
        &clock,
        Schedule::with_timeout(secs(250)),
    )
    .expect("the fake never fails");

    assert_eq!(
        *clock.sleeps.borrow(),
        vec![secs(10), secs(20), secs(40), secs(60), secs(60), secs(60)],
        "10 s, doubling, capped at 60 s, the last ending on the 250 s deadline"
    );
    assert!(watched.timed_out, "still pending at the deadline");
    assert_eq!(
        watched.polls, 7,
        "one poll after each sleep, the last at the deadline"
    );
    assert_eq!(watched.waited, secs(250), "waited exactly the timeout");
}

#[test]
fn a_zero_timeout_polls_once_and_times_out() {
    let clock = FakeClock::default();

    let watched = watch(
        &Script::new(vec![pending()]),
        &clock,
        Schedule::with_timeout(secs(0)),
    )
    .expect("the fake never fails");

    assert_eq!(
        (watched.polls, watched.timed_out),
        (1, true),
        "one poll, then timeout"
    );
    assert_eq!(*clock.sleeps.borrow(), Vec::<Duration>::new(), "no sleep");
}

#[test]
fn a_blocker_returns_at_once_even_with_work_pending() {
    let clock = FakeClock::default();
    let script = Script::new(vec![pending(), failing()]);

    let watched =
        watch(&script, &clock, Schedule::with_timeout(secs(1800))).expect("the fake never fails");

    assert_eq!(watched.polls, 2, "the second poll found the failure");
    assert!(!watched.timed_out, "a blocker settles the watch");
    assert_eq!(
        watched.snapshot,
        failing(),
        "the blocking snapshot is reported"
    );
    assert_eq!(
        *clock.sleeps.borrow(),
        vec![secs(10)],
        "one back-off before it"
    );
}

#[test]
fn a_last_sleep_shorter_than_the_back_off_ends_on_the_deadline() {
    let clock = FakeClock::default();

    let watched = watch(
        &Script::new(vec![pending()]),
        &clock,
        Schedule::with_timeout(secs(15)),
    )
    .expect("the fake never fails");

    assert_eq!(
        *clock.sleeps.borrow(),
        vec![secs(10), secs(5)],
        "cut to what is left"
    );
    assert_eq!(
        watched.polls, 3,
        "a poll at the start, after 10 s and at 15 s"
    );
}
