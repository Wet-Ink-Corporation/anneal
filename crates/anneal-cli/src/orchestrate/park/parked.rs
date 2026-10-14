//! Recording a park in the ledger: at most one `weighin_parked` per item and ticket, also when
//! two parks race.
//!
//! A park reads the ticket's `weighin_parked` events and appends only on the condition that no
//! park of the ticket landed after that read (`Store::append_if`, as `round` and `migrate` do).
//! On a conflict it reads again: the item parked meanwhile means it is recorded already, and
//! another item's park means it tries again, at most [`PARK_ATTEMPTS`] times.

use anneal_ledger::{
    Appended, Condition, EventEnvelope, EventKind, Filter, LedgerEvent, LedgerTag, Recorded, Store,
    TagError, TagKey,
};

use crate::error::CliError;

/// How many times a park reads and appends again when another park of its ticket landed in
/// between.
const PARK_ATTEMPTS: usize = 4;

/// What recording a park came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Parked {
    /// Appended at this position.
    Recorded(u64),
    /// The ledger had this park of the item at this position; nothing was appended.
    Already(u64),
}

/// What one try came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Attempt {
    /// The park is recorded, by this try or an earlier writer.
    Done(Parked),
    /// Another item's park of the ticket landed after the read; read again.
    Retry,
}

/// The `weighin_parked` events of `ticket` (its `task:` subject tag).
///
/// # Errors
/// [`TagError`] when `ticket` cannot be a tag value.
pub(super) fn parks_of(ticket: &str) -> Result<Filter, TagError> {
    Ok(Filter {
        types: vec![EventKind::WeighinParked],
        tags: vec![LedgerTag::new(TagKey::Task, ticket)?],
    })
}

/// A park to record: `event`, the `weighin_parked` of `item`, one of the events `parks` matches.
#[derive(Debug, Clone, Copy)]
pub(super) struct Park<'a> {
    /// The ticket's parks.
    pub(super) parks: &'a Filter,
    /// The item parked.
    pub(super) item: &'a str,
    /// Its `weighin_parked` envelope.
    pub(super) event: &'a EventEnvelope,
}

impl Park<'_> {
    /// Appends the park unless one of its item is recorded already.
    ///
    /// # Errors
    /// The store's, or [`CliError::ParkContended`] when every try lost to another item's park.
    pub(super) fn record(&self, store: &Store) -> Result<Parked, CliError> {
        for _ in 0..PARK_ATTEMPTS {
            let seen = store.query(self.parks)?;
            if let Attempt::Done(parked) = self.try_once(store, &seen)? {
                return Ok(parked);
            }
        }
        Err(CliError::ParkContended {
            attempts: PARK_ATTEMPTS,
        })
    }

    /// Decides from `seen`, the parks read, then appends on the condition that none followed.
    fn try_once(&self, store: &Store, seen: &[Recorded]) -> Result<Attempt, CliError> {
        if let Some(position) = position_of(seen, self.item) {
            return Ok(Attempt::Done(Parked::Already(position)));
        }
        let last = seen.last().map_or(0, |recorded| recorded.position);
        // The condition owns its filter; `parks` is kept to read the winner back.
        let condition = Condition::none_matching(self.parks.clone()).after(last);
        match store.append_if(self.event, &condition)? {
            Appended::At(position) => Ok(Attempt::Done(Parked::Recorded(position))),
            Appended::Conflict => Ok(position_of(&store.query(self.parks)?, self.item)
                .map_or(Attempt::Retry, |position| {
                    Attempt::Done(Parked::Already(position))
                })),
        }
    }
}

/// The position of the park of `item` among `parks`.
fn position_of(parks: &[Recorded], item: &str) -> Option<u64> {
    parks.iter().find_map(|recorded| {
        matches!(&recorded.event.event, LedgerEvent::WeighinParked(parked) if parked.item_id == item)
            .then_some(recorded.position)
    })
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use anneal_ledger::event::WeighinParked;
    use anneal_ledger::{EventEnvelope, LedgerEvent, Store, Timestamp};

    use super::{Attempt, Park, Parked, parks_of};

    type TestResult = Result<(), Box<dyn Error>>;

    fn park(item: &str) -> Result<EventEnvelope, Box<dyn Error>> {
        let event = LedgerEvent::WeighinParked(WeighinParked {
            item_id: item.to_owned(),
            ticket: "7".to_owned(),
            decision: None,
        });
        Ok(EventEnvelope::new(event, "cli", Timestamp::now()?))
    }

    #[test]
    fn a_racing_park_of_the_same_item_after_the_read_appends_nothing() -> TestResult {
        let dir = tempfile::tempdir()?;
        let store = Store::open(&dir.path().join("ledger.sqlite3"))?;
        let parks = parks_of("7")?;
        let seen = store.query(&parks)?;
        let racing = store.append(&park("wi-e41e02")?)?;

        let event = park("wi-e41e02")?;
        let mine = Park {
            parks: &parks,
            item: "wi-e41e02",
            event: &event,
        };

        let attempt = mine.try_once(&store, &seen)?;

        assert_eq!(attempt, Attempt::Done(Parked::Already(racing)));
        assert_eq!(store.query(&parks)?.len(), 1, "exactly one weighin_parked");
        Ok(())
    }

    #[test]
    fn a_racing_park_of_another_item_after_the_read_is_read_again_and_recorded() -> TestResult {
        let dir = tempfile::tempdir()?;
        let store = Store::open(&dir.path().join("ledger.sqlite3"))?;
        let parks = parks_of("7")?;
        let seen = store.query(&parks)?;
        store.append(&park("wi-0a0a0a")?)?;

        let event = park("wi-e41e02")?;
        let mine = Park {
            parks: &parks,
            item: "wi-e41e02",
            event: &event,
        };

        let stale = mine.try_once(&store, &seen)?;
        let recorded = mine.record(&store)?;

        assert_eq!(stale, Attempt::Retry, "nothing appended on a stale read");
        assert_eq!(recorded, Parked::Recorded(2));
        assert_eq!(store.query(&parks)?.len(), 2);
        Ok(())
    }

    #[test]
    fn a_park_of_the_same_item_for_another_ticket_is_not_this_ticket_s() -> TestResult {
        let dir = tempfile::tempdir()?;
        let store = Store::open(&dir.path().join("ledger.sqlite3"))?;
        let other = LedgerEvent::WeighinParked(WeighinParked {
            item_id: "wi-e41e02".to_owned(),
            ticket: "8".to_owned(),
            decision: None,
        });
        store.append(&EventEnvelope::new(other, "cli", Timestamp::now()?))?;

        let parks = parks_of("7")?;
        let event = park("wi-e41e02")?;
        let mine = Park {
            parks: &parks,
            item: "wi-e41e02",
            event: &event,
        };

        let recorded = mine.record(&store)?;

        assert_eq!(recorded, Parked::Recorded(2));
        Ok(())
    }
}
