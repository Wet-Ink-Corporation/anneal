//! anneal's event ledger: what happened in a repository's sessions (tests run, tasks done,
//! plans approved, gates blocked), kept in one SQLite event store per repository.
//!
//! - [`event`]: the versioned registry of event types and their payloads.
//! - [`tag`]: the `key:value` tag vocabulary events are indexed by.
//! - [`envelope`]: the envelope every event is stored in, and strict parsing of it.
//! - [`store`]: happenstance's SQLite event store under a TRUNCATE journal, with plain and
//!   conditioned (first-writer-wins) appends, queries and tails.
//!
//! The ledger knows nothing about git or the environment: callers resolve the store's path
//! (the `anneal` CLI uses `<git-common-dir>/anneal/ledger.sqlite3`) and hand it to
//! [`Store::open`].
//!
//! # Examples
//!
//! ```
//! use anneal_ledger::event::TaskDone;
//! use anneal_ledger::{EventEnvelope, EventKind, Filter, LedgerEvent, Store, Timestamp};
//!
//! let dir = tempfile::tempdir()?;
//! let store = Store::open(&dir.path().join("anneal/ledger.sqlite3"))?;
//! let done = LedgerEvent::TaskDone(TaskDone { task: "T1".to_owned() });
//!
//! let position = store.append(&EventEnvelope::new(done, "cli", Timestamp::now()?))?;
//!
//! let filter = Filter { types: vec![EventKind::TaskDone], ..Filter::default() };
//! let events = store.query(&filter)?;
//! assert_eq!((position, events.len()), (1, 1));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod envelope;
pub mod event;
pub mod store;
pub mod tag;

pub use envelope::{EventEnvelope, SchemaError, Timestamp, TimestampError};
pub use event::{EventKind, LedgerEvent, REGISTRY_VERSION, UnknownEventKind};
pub use store::{Appended, Condition, Filter, OpenError, Recorded, Store, StoreError};
pub use tag::{LedgerTag, TagError, TagKey};
