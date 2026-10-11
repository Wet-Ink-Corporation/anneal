//! The ledger store: happenstance's SQLite event store, opened under a TRUNCATE journal.
//!
//! # Journal mode
//!
//! happenstance opens its stores in WAL, and SQLite's own default is DELETE. Both delete files
//! beside the database (WAL its `-wal` and `-shm`, DELETE its journal at every commit), and
//! Cowork's connected folders refuse deletes and WAL's shared memory. So the ledger runs under
//! TRUNCATE: one `-journal` beside it, emptied at each commit and never deleted. The ledger
//! writes a few events per turn and does not need WAL's concurrency.
//!
//! [`Store::open`] therefore never calls `SqliteEventStore::open` on the ledger. It opens its
//! own connection and, for a new file, copies the schema happenstance builds in a template on
//! the local temporary disk, inside one `BEGIN IMMEDIATE`, so two processes creating the same
//! ledger cannot both write a schema.
//!
//! # Schema version
//!
//! The ledger's schema version is `PRAGMA user_version` (1: happenstance 0.3.2's schema under
//! TRUNCATE); happenstance keeps its own counter in `store_meta`. A new ledger is stamped in
//! the transaction that copies its schema. A ledger stamped by a newer build is refused and
//! left untouched.
//!
//! # Concurrency
//!
//! Every append runs in happenstance's own `BEGIN IMMEDIATE`, and each connection waits for
//! another's lock, so appends from concurrent hooks serialise. [`Store::append_if`] adds a DCB
//! condition checked inside that same transaction: first writer wins.

mod schema;

use std::fmt;
use std::path::{Path, PathBuf};

use happenstance_core::{
    AppendCondition, AppendError, Event, InvalidEventType, InvalidQuery, Query, QueryItem,
    ReadOptions, SendEventStore, SequencePosition, SequencedEvent, Tags, collect,
};
use happenstance_sqlite::event_store::{SqliteEventStore, SqliteEventStoreError};
use happenstance_sqlite::rusqlite;
use serde::Serialize;

use crate::envelope::{EventEnvelope, SchemaError};
use crate::event::EventKind;
use crate::tag::LedgerTag;

/// Why a ledger file could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    /// The ledger's directory could not be created.
    #[error("creating the ledger's directory")]
    CreateDir(#[source] std::io::Error),
    /// SQLite failed.
    #[error("SQLite failed")]
    Sqlite(#[from] rusqlite::Error),
    /// SQLite would not run the ledger under a TRUNCATE journal.
    #[error("SQLite kept journal mode {found}, not truncate; another process may hold it in WAL")]
    JournalMode {
        /// The mode SQLite reported.
        found: String,
    },
    /// The ledger was stamped by a newer build.
    #[error(
        "the ledger has schema {found}, newer than this anneal reads ({supported}); upgrade anneal"
    )]
    NewerSchema {
        /// The version in the file.
        found: i64,
        /// The version this build reads.
        supported: i64,
    },
    /// The ledger carries a schema version this build has no way to read.
    #[error("the ledger has schema version {found}, which this anneal cannot read")]
    UnknownSchema {
        /// The version in the file.
        found: i64,
    },
    /// The temporary directory for the schema template could not be created.
    #[error("creating a temporary directory for the schema template")]
    TempDir(#[source] std::io::Error),
    /// happenstance could not build the schema template.
    #[error("building happenstance's schema template")]
    Template(#[source] SqliteEventStoreError),
    /// happenstance refused the ledger's connection.
    #[error("happenstance refused the ledger")]
    Store(#[source] SqliteEventStoreError),
    /// The async runtime that drives happenstance could not start.
    #[error("starting the async runtime")]
    Runtime(#[source] std::io::Error),
    /// A step failed, and undoing it (rollback, detach) failed too.
    #[error("cleaning up after a failure also failed ({cleanup})")]
    Cleanup {
        /// The step that failed first.
        #[source]
        failure: Box<OpenError>,
        /// The cleanup's own failure.
        cleanup: rusqlite::Error,
    },
}

/// Why a ledger operation failed.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The ledger could not be opened.
    #[error("opening the ledger {}", path.display())]
    Open {
        /// The ledger file.
        path: PathBuf,
        /// What went wrong.
        #[source]
        source: OpenError,
    },
    /// The event to append does not match the ledger schema (an invalid tag value).
    #[error("the event does not match the ledger schema")]
    Schema(#[from] SchemaError),
    /// The event could not be encoded.
    #[error("encoding the event")]
    Encode(#[source] serde_json::Error),
    /// happenstance refused the append.
    #[error("appending to the ledger")]
    Append(#[source] AppendError<SqliteEventStoreError>),
    /// happenstance could not read the ledger.
    #[error("reading the ledger")]
    Read(#[source] SqliteEventStoreError),
    /// A stored event is not a registered event.
    #[error("the event at position {position} is not a registered ledger event")]
    Decode {
        /// Where it is stored.
        position: u64,
        /// Why it does not decode.
        #[source]
        source: serde_json::Error,
    },
    /// An event type name was refused by the store. Registered names are valid store types
    /// (tested), so this means the registry and the store disagree.
    #[error("the store refused the event type")]
    EventType(#[source] InvalidEventType),
    /// The filter could not be turned into a store query.
    #[error("building the ledger query")]
    Query(#[source] InvalidQuery),
}

/// Which events to read: any of `types` (all types when empty) carrying every tag in `tags`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    /// Event types to match; empty matches every type.
    pub types: Vec<EventKind>,
    /// Tags an event must all carry; empty matches every event.
    pub tags: Vec<LedgerTag>,
}

impl Filter {
    fn to_query(&self) -> Result<Query, InvalidQuery> {
        if self.types.is_empty() && self.tags.is_empty() {
            return Ok(Query::all());
        }
        let types = self.types.iter().map(|kind| kind.as_str());
        // The query owns its tags, so they are copied out of the borrowed filter.
        let tags: Tags = self.tags.iter().map(|tag| tag.as_tag().clone()).collect();
        QueryItem::new(types, tags).map(Query::from_item)
    }
}

/// The condition of a first-writer-wins append: no event matching `filter` may exist after
/// `after` (anywhere in the ledger by default).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Condition {
    filter: Filter,
    after: Option<u64>,
}

impl Condition {
    /// Fails the append when any event matches `filter`.
    #[must_use]
    pub const fn none_matching(filter: Filter) -> Self {
        Self {
            filter,
            after: None,
        }
    }

    /// Only events after `position` (exclusive) count: the caller read the ledger up to
    /// `position` and decided from that. Position 0 means the whole ledger.
    #[must_use]
    pub const fn after(mut self, position: u64) -> Self {
        self.after = Some(position);
        self
    }

    fn to_append_condition(&self) -> Result<AppendCondition, InvalidQuery> {
        let after = self.after.and_then(SequencePosition::new);
        Ok(AppendCondition::new(self.filter.to_query()?).after_opt(after))
    }
}

/// The outcome of a conditioned append.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appended {
    /// Written at this position.
    At(u64),
    /// Not written: an event matching the condition exists. Someone else wrote first.
    Conflict,
}

/// An event as stored: its position (from 1, in append order) and its envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Recorded {
    /// Where the event sits in the ledger, from 1.
    pub position: u64,
    /// The event.
    pub event: EventEnvelope,
}

/// An open ledger.
pub struct Store {
    inner: SqliteEventStore,
    runtime: tokio::runtime::Runtime,
}

impl fmt::Debug for Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Store").finish_non_exhaustive()
    }
}

impl Store {
    /// Opens the ledger at `path`, creating it and its directory when absent.
    ///
    /// # Errors
    /// [`StoreError::Open`] when the file cannot be created or opened under TRUNCATE, or was
    /// stamped by a newer build ([`OpenError::NewerSchema`]).
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        Self::try_open(path).map_err(|source| StoreError::Open {
            path: path.to_path_buf(),
            source,
        })
    }

    fn try_open(path: &Path) -> Result<Self, OpenError> {
        if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(OpenError::CreateDir)?;
        }
        let inner = schema::open(path)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(OpenError::Runtime)?;
        Ok(Self { inner, runtime })
    }

    /// Appends `event` and returns its position.
    ///
    /// # Errors
    /// [`StoreError::Schema`] for an invalid tag value, [`StoreError::Append`] when the store
    /// refuses the write (including a lock held past the busy timeout).
    pub fn append(&self, event: &EventEnvelope) -> Result<u64, StoreError> {
        let stored = stored_event(event)?;
        self.runtime
            .block_on(self.inner.append(&[stored], None))
            .map(u64::from)
            .map_err(StoreError::Append)
    }

    /// Appends `event` unless an event matching `condition` exists: first writer wins. The
    /// condition is checked inside the append's transaction, so two writers racing on the same
    /// condition cannot both succeed.
    ///
    /// # Errors
    /// As [`Store::append`]; a matching event is [`Appended::Conflict`], not an error.
    pub fn append_if(
        &self,
        event: &EventEnvelope,
        condition: &Condition,
    ) -> Result<Appended, StoreError> {
        let stored = stored_event(event)?;
        let condition = condition.to_append_condition().map_err(StoreError::Query)?;
        match self
            .runtime
            .block_on(self.inner.append(&[stored], Some(&condition)))
        {
            Ok(position) => Ok(Appended::At(u64::from(position))),
            Err(AppendError::ConditionViolated(_)) => Ok(Appended::Conflict),
            Err(refused) => Err(StoreError::Append(refused)),
        }
    }

    /// Every event matching `filter`, in position order.
    ///
    /// # Errors
    /// [`StoreError::Read`] when the store cannot be read, [`StoreError::Decode`] when a stored
    /// event is not registered in this build.
    pub fn query(&self, filter: &Filter) -> Result<Vec<Recorded>, StoreError> {
        self.read(filter, ReadOptions::new())
    }

    /// The last `n` events matching `filter`, in position order.
    ///
    /// # Errors
    /// As [`Store::query`].
    pub fn tail(&self, filter: &Filter, n: usize) -> Result<Vec<Recorded>, StoreError> {
        let mut events = self.read(filter, ReadOptions::new().backwards().limit(n))?;
        events.reverse();
        Ok(events)
    }

    fn read(&self, filter: &Filter, options: ReadOptions) -> Result<Vec<Recorded>, StoreError> {
        let query = filter.to_query().map_err(StoreError::Query)?;
        let stored = self
            .runtime
            .block_on(collect(self.inner.read(&query, options)))
            .map_err(StoreError::Read)?;
        stored.iter().map(decode).collect()
    }
}

/// The happenstance event for `envelope`: its type, its JSON, and its tags.
fn stored_event(envelope: &EventEnvelope) -> Result<Event, StoreError> {
    let tags: Tags = envelope
        .tags()
        .map_err(SchemaError::Tag)?
        .into_iter()
        .map(LedgerTag::into_tag)
        .collect();
    let data = envelope.to_json().map_err(StoreError::Encode)?;
    Ok(Event::new(envelope.event.kind().as_str(), data)
        .map_err(StoreError::EventType)?
        .with_tags(tags))
}

/// Reads a stored event back. Reading is lenient about keys a newer build added (they are
/// dropped); writing is strict ([`EventEnvelope::from_value`]).
fn decode(stored: &SequencedEvent) -> Result<Recorded, StoreError> {
    let position = u64::from(stored.position);
    serde_json::from_slice(stored.event.data())
        .map(|event| Recorded { position, event })
        .map_err(|source| StoreError::Decode { position, source })
}

#[cfg(test)]
mod tests;
