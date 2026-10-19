//! `anneal ledger`: append to, query, tail and journal the event ledger.
//!
//! The ledger itself (event registry, envelope, SQLite store) is the `anneal-ledger` crate;
//! this module parses the command line, resolves the store ([`paths::ledger_store`]) and
//! writes the result. A bad event exits 2 with an envelope and nothing is written. A `by` on
//! `plan_approved` or `verdict_set` is a usage error (2, nothing on stdout, nothing written):
//! `plan approve` and `verdict set` write it.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use anneal_ledger::event::{PlanApproved, VerdictSet};
use anneal_ledger::{
    EventEnvelope, EventKind, Filter, LedgerEvent, LedgerTag, REGISTRY_VERSION, Recorded, Store,
    StoreError, TagError, TagKey, Timestamp,
};
use clap::{Args, Subcommand};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::error_chain;
use crate::exit_codes::{self, Code};
use crate::paths::{self, StoreLocation, StoreSource};

/// `anneal ledger`.
#[derive(Debug, Args)]
pub struct LedgerArgs {
    /// Use this ledger file instead of the resolved one
    #[arg(long, global = true, value_name = "FILE")]
    store: Option<PathBuf>,
    #[command(subcommand)]
    action: LedgerAction,
}

/// The `anneal ledger` subcommands.
#[derive(Debug, Subcommand)]
pub enum LedgerAction {
    /// Append one event, given as a JSON object with a registered `type`
    Append(AppendArgs),
    /// Print the events matching the filters, oldest first
    Query(FilterArgs),
    /// Print the last events matching the filters, oldest first
    Tail(TailArgs),
    /// Print every event as one JSON line, for committing a journal
    Journal,
}

/// `anneal ledger append`.
#[derive(Debug, Args)]
pub struct AppendArgs {
    #[arg(
        value_name = "EVENT_JSON",
        help = r#"The event, e.g. '{"type":"task_done","task":"T1"}'"#
    )]
    event: String,
    /// The writer, when the event names none
    #[arg(long, default_value = "cli")]
    actor: String,
    /// The session id, when the event names none
    #[arg(long)]
    session: Option<String>,
}

/// Filters shared by `query` and `tail`: any of the types, all of the tags.
#[derive(Debug, Args)]
pub struct FilterArgs {
    /// Only events of this type (repeatable: any of them)
    #[arg(long = "type", value_name = "TYPE")]
    types: Vec<EventKind>,
    /// Only events carrying this key:value tag (repeatable: all of them)
    #[arg(long = "tag", value_name = "KEY:VALUE")]
    tags: Vec<LedgerTag>,
    /// Only events of this session (the tag session:ID)
    #[arg(long, value_name = "ID", value_parser = session_tag)]
    session: Option<LedgerTag>,
}

/// `anneal ledger tail`.
#[derive(Debug, Args)]
pub struct TailArgs {
    /// How many events
    #[arg(short = 'n', long = "lines", value_name = "N", default_value_t = 10)]
    lines: usize,
    #[command(flatten)]
    filter: FilterArgs,
}

fn session_tag(id: &str) -> Result<LedgerTag, TagError> {
    LedgerTag::new(TagKey::Session, id)
}

impl FilterArgs {
    fn to_filter(&self) -> Filter {
        // The filter owns its tags; the parsed arguments stay with clap's tree.
        let tags = self.tags.iter().chain(&self.session).cloned().collect();
        Filter {
            types: self.types.clone(),
            tags,
        }
    }
}

/// Runs `anneal ledger <action>` in `cwd`, reading environment variables through `lookup`.
pub(crate) fn run<O: Write>(
    args: &LedgerArgs,
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
    out: &mut O,
) -> Result<Code, CliError> {
    let locate = || paths::ledger_store(args.store.as_deref(), &lookup, cwd);
    match &args.action {
        LedgerAction::Append(append_args) => append(append_args, locate, out),
        LedgerAction::Query(filter) => {
            let filter = filter.to_filter();
            read(&locate()?, "query", |store| store.query(&filter), out)
        }
        LedgerAction::Tail(tail) => {
            let filter = tail.filter.to_filter();
            read(
                &locate()?,
                "tail",
                |store| store.tail(&filter, tail.lines),
                out,
            )
        }
        LedgerAction::Journal => journal(&locate()?, out),
    }
}

#[derive(Serialize)]
struct Appended<'a> {
    store: &'a Path,
    source: StoreSource,
    position: u64,
    event: &'a EventEnvelope,
}

#[derive(Serialize)]
struct Listing<'a> {
    store: &'a Path,
    source: StoreSource,
    registry: u32,
    events: &'a [Recorded],
}

fn append<O: Write>(
    args: &AppendArgs,
    locate: impl FnOnce() -> Result<StoreLocation, paths::PathError>,
    out: &mut O,
) -> Result<Code, CliError> {
    const SUBCOMMAND: &str = "append";
    let event = match parse_event(args)? {
        Ok(event) => event,
        Err(schema) => return schema_error(SUBCOMMAND, &schema, out),
    };
    refuse_a_claimed_giver(&event.event)?;
    let location = locate()?;
    let position = Store::open(&location.path)?.append(&event)?;
    let data = Appended {
        store: &location.path,
        source: location.source,
        position,
        event: &event,
    };
    emit(
        &Envelope::new(CommandName::Ledger, Some(SUBCOMMAND), exit_codes::OK, data),
        out,
    )
}

/// The event in `args`, with `actor`, `session_id` and `ts` filled in where it leaves them
/// out. The outer `Result` is a failure to run (the clock); the inner one a schema error.
fn parse_event(
    args: &AppendArgs,
) -> Result<Result<EventEnvelope, anneal_ledger::SchemaError>, CliError> {
    let mut value: Value = match serde_json::from_str(&args.event) {
        Ok(value) => value,
        Err(error) => return Ok(Err(anneal_ledger::SchemaError::Json(error))),
    };
    if let Value::Object(fields) = &mut value {
        fill(fields, "actor", || Ok(Value::from(args.actor.as_str())))?;
        if let Some(session) = &args.session {
            fill(fields, "session_id", || Ok(Value::from(session.as_str())))?;
        }
        fill(fields, "ts", || Ok(Value::from(Timestamp::now()?.as_str())))?;
    }
    Ok(EventEnvelope::from_value(&value))
}

/// Refuses a `by` on `plan_approved` or `verdict_set` (#62): who gave an approval or a verdict
/// is recorded only by their writers, `plan approve` and `verdict set`. For `verdict_set` that
/// is where the guard's `human-only` rule sees the actor, so a raw append cannot claim a human
/// verdict; `plan approve` is allowed with any actor (wi-720468), so for `plan_approved` the
/// refusal only keeps one writer of `by`.
fn refuse_a_claimed_giver(event: &LedgerEvent) -> Result<(), CliError> {
    let (kind, writer) = match event {
        LedgerEvent::PlanApproved(PlanApproved { by: Some(_), .. }) => {
            (EventKind::PlanApproved, "plan approve")
        }
        LedgerEvent::VerdictSet(VerdictSet { by: Some(_), .. }) => {
            (EventKind::VerdictSet, "verdict set")
        }
        _ => return Ok(()),
    };
    Err(CliError::Usage {
        reason: format!(
            "ledger append does not write a `by` on {kind}: use `anneal {writer} --actor/--via`"
        ),
    })
}

/// Sets `key` to `make()` when it is absent or `null`.
fn fill(
    fields: &mut Map<String, Value>,
    key: &str,
    make: impl FnOnce() -> Result<Value, CliError>,
) -> Result<(), CliError> {
    if fields.get(key).is_none_or(Value::is_null) {
        fields.insert(key.to_owned(), make()?);
    }
    Ok(())
}

fn read<O: Write>(
    location: &StoreLocation,
    subcommand: &str,
    select: impl FnOnce(&Store) -> Result<Vec<Recorded>, StoreError>,
    out: &mut O,
) -> Result<Code, CliError> {
    let events = match select_events(location, subcommand, select, out)? {
        Ok(events) => events,
        Err(code) => return Ok(code),
    };
    let data = Listing {
        store: &location.path,
        source: location.source,
        registry: REGISTRY_VERSION,
        events: &events,
    };
    emit(
        &Envelope::new(CommandName::Ledger, Some(subcommand), exit_codes::OK, data),
        out,
    )
}

/// Runs `select` on the store at `location`. A stored event this build cannot decode is a
/// schema error: its envelope is written and its code returned as `Err`.
fn select_events<O: Write>(
    location: &StoreLocation,
    subcommand: &str,
    select: impl FnOnce(&Store) -> Result<Vec<Recorded>, StoreError>,
    out: &mut O,
) -> Result<Result<Vec<Recorded>, Code>, CliError> {
    match select(&Store::open(&location.path)?) {
        Ok(events) => Ok(Ok(events)),
        Err(decode @ StoreError::Decode { .. }) => schema_error(subcommand, &decode, out).map(Err),
        Err(other) => Err(other.into()),
    }
}

/// Every event's envelope as one JSON line, in position order. Not wrapped in the result
/// envelope: the output is the journal file itself.
fn journal<O: Write>(location: &StoreLocation, out: &mut O) -> Result<Code, CliError> {
    let all = |store: &Store| store.query(&Filter::default());
    let events = match select_events(location, "journal", all, out)? {
        Ok(events) => events,
        Err(code) => return Ok(code),
    };
    for recorded in &events {
        serde_json::to_writer(&mut *out, &recorded.event)
            .map_err(|error| CliError::Output(error.into()))?;
        out.write_all(b"\n").map_err(CliError::Output)?;
    }
    Ok(exit_codes::OK)
}

/// The envelope for an event that does not match the schema: exit 2, one finding.
fn schema_error<O: Write>(
    subcommand: &str,
    error: &dyn std::error::Error,
    out: &mut O,
) -> Result<Code, CliError> {
    let findings = [Finding::new("ledger/schema", error_chain::reason(error))];
    let types: Vec<&str> = EventKind::ALL.iter().map(|kind| kind.as_str()).collect();
    let data = serde_json::json!({"registry": REGISTRY_VERSION, "types": types});
    let envelope = Envelope::new(
        CommandName::Ledger,
        Some(subcommand),
        exit_codes::LEDGER_SCHEMA_ERROR,
        data,
    );
    emit(&envelope.with_findings(&findings), out)
}

fn emit<D: Serialize, O: Write>(envelope: &Envelope<'_, D>, out: &mut O) -> Result<Code, CliError> {
    envelope.emit(out).map_err(CliError::Output)
}

#[cfg(test)]
mod tests {
    use super::{AppendArgs, parse_event};

    fn args(event: &str, session: Option<&str>) -> AppendArgs {
        AppendArgs {
            event: event.to_owned(),
            actor: "cli".to_owned(),
            session: session.map(str::to_owned),
        }
    }

    #[test]
    fn append_fills_actor_session_and_ts_only_where_the_event_leaves_them_out() {
        let given = args(
            r#"{"type":"task_done","task":"T1","actor":"hook","session_id":null}"#,
            Some("s9"),
        );

        let event = parse_event(&given)
            .expect("the clock works")
            .expect("a valid event");

        assert_eq!(event.actor, "hook", "the event's own actor wins");
        assert_eq!(event.session_id.as_deref(), Some("s9"), "null is filled");
        assert!(event.ts.as_str().ends_with('Z'), "ts filled with now");
    }
}
