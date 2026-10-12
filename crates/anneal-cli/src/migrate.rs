//! `anneal migrate start|done|contract`: an expand, migrate, contract migration tracked wave by
//! wave in the ledger.
//!
//! `start --migration <id> --wave <n>` appends `migrate.wave.started`, `done` appends
//! `migrate.wave.done`, and `contract --migration <id>` appends `migrate.contract`, each tagged
//! `migration:<id>`. A step is judged against the migration's events (`waves`): a wave starts
//! once and is done only while open; contract needs a wave done and none open; nothing follows
//! a contract. A refused step appends nothing: exit 1 when contract finds a wave open, 3 for
//! any other refusal. The append is first-writer-wins on the migration's events after those
//! read, so two racing steps cannot both land. Like `mode`, it has no `--store`: it writes to
//! the ledger `$ANNEAL_STORE`, the git common dir or the folder gives. See `docs/migrate.md`.

pub(crate) mod waves;

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use anneal_ledger::{
    Appended, Condition, EventEnvelope, Filter, LedgerTag, Store, TagKey, Timestamp,
};
use clap::{Args, Subcommand};
use serde::Serialize;

use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::exit_codes::{self, Code};
use crate::paths;
use waves::{Refusal, Step, Waves};

/// The most events one migration may have; more is an error rather than a partial fold
/// (H-13).
pub const MAX_MIGRATION_EVENTS: usize = 4096;

/// `anneal migrate`.
#[derive(Debug, Args)]
pub struct MigrateArgs {
    #[command(subcommand)]
    action: MigrateAction,
}

/// The `anneal migrate` subcommands.
#[derive(Debug, Subcommand)]
pub enum MigrateAction {
    /// Record that a wave of the migration started
    Start(WaveArgs),
    /// Record that a wave of the migration is done
    Done(WaveArgs),
    /// Record the contract step; refused while any wave is open
    Contract(ContractArgs),
}

/// `anneal migrate start|done`.
#[derive(Debug, Args)]
pub struct WaveArgs {
    /// The migration's id
    #[arg(long, value_name = "ID")]
    migration: String,
    /// The wave's number (from 1)
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..))]
    wave: u32,
    /// The session id to record
    #[arg(long, value_name = "ID")]
    session: Option<String>,
}

/// `anneal migrate contract`.
#[derive(Debug, Args)]
pub struct ContractArgs {
    /// The migration's id
    #[arg(long, value_name = "ID")]
    migration: String,
    /// The session id to record
    #[arg(long, value_name = "ID")]
    session: Option<String>,
}

/// One requested step: the migration, the step and the session.
#[derive(Debug, Clone, Copy)]
struct Request<'a> {
    migration: &'a str,
    step: Step,
    session: Option<&'a String>,
}

impl<'a> From<&'a MigrateAction> for Request<'a> {
    fn from(action: &'a MigrateAction) -> Self {
        let wave = |args: &'a WaveArgs, step: Step| Self {
            migration: &args.migration,
            step,
            session: args.session.as_ref(),
        };
        match action {
            MigrateAction::Start(args) => wave(args, Step::Start(args.wave)),
            MigrateAction::Done(args) => wave(args, Step::Done(args.wave)),
            MigrateAction::Contract(args) => Self {
                migration: &args.migration,
                step: Step::Contract,
                session: args.session.as_ref(),
            },
        }
    }
}

/// Runs `anneal migrate <action>` in `cwd`, reading environment variables through `lookup`.
pub(crate) fn run<O: Write>(
    args: &MigrateArgs,
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
    out: &mut O,
) -> Result<Code, CliError> {
    let request = Request::from(&args.action);
    let mut envelope = EventEnvelope::new(
        // The event stores the migration's id.
        request.step.event(request.migration.to_owned()),
        "cli",
        Timestamp::now()?,
    );
    envelope.session_id = request.session.cloned();
    // The migration and session become ledger tags; check them before a ledger is opened (or
    // created), so a value the store would refuse is a usage error that leaves no file.
    let tag = LedgerTag::new(TagKey::Migration, request.migration)
        .and_then(|tag| envelope.tags().map(|_| tag))
        .map_err(|error| CliError::Usage {
            reason: format!("the --migration and --session are recorded as ledger tags: {error}"),
        })?;
    let location = paths::ledger_store(None, lookup, cwd)?;
    let filter = Filter {
        types: Vec::new(),
        tags: vec![tag],
    };
    let existing = open_if_present(&location.path)?;
    let (state, last) = match &existing {
        Some(store) => read(store, &filter, request.migration)?,
        None => (Waves::default(), 0),
    };
    let report = Report {
        request,
        store: &location.path,
    };
    if let Err(refusal) = state.check(request.step) {
        return report.refused(&state, &refusal, out);
    }
    let store = match existing {
        Some(store) => store,
        None => Store::open(&location.path)?,
    };
    match store.append_if(&envelope, &Condition::none_matching(filter).after(last))? {
        Appended::At(position) => report.recorded(&state.after(request.step), position, out),
        Appended::Conflict => report.refused(&state, &Refusal::Concurrent, out),
    }
}

/// The ledger at `path` when its file exists; `None` when it does not, so a refused step
/// creates no ledger.
fn open_if_present(path: &Path) -> Result<Option<Store>, CliError> {
    match fs::metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        _ => Ok(Some(Store::open(path)?)),
    }
}

/// The migration's state from its events in `store`, and the position of its latest event (0
/// with none).
fn read(store: &Store, filter: &Filter, migration: &str) -> Result<(Waves, u64), CliError> {
    let records = store.tail(filter, MAX_MIGRATION_EVENTS.saturating_add(1))?;
    if records.len() > MAX_MIGRATION_EVENTS {
        return Err(CliError::Migrate(MigrateError::TooManyEvents {
            migration: migration.to_owned(),
        }));
    }
    let last = records.last().map_or(0, |record| record.position);
    let state = Waves::fold(records.iter().map(|record| &record.event.event));
    Ok((state, last))
}

/// Why `anneal migrate` could not judge a step.
#[derive(Debug, thiserror::Error)]
pub enum MigrateError {
    /// The migration has more events than are read.
    #[error("migrate: migration {migration:?} has more than {MAX_MIGRATION_EVENTS} events")]
    TooManyEvents {
        /// The migration's id.
        migration: String,
    },
}

impl MigrateError {
    /// The exit code: 70, the ledger holds more than the command reads.
    #[must_use]
    pub const fn code(&self) -> Code {
        match self {
            Self::TooManyEvents { .. } => exit_codes::INTERNAL,
        }
    }
}

/// What a run reports with: the request and the ledger it read.
#[derive(Debug, Clone, Copy)]
struct Report<'a> {
    request: Request<'a>,
    store: &'a Path,
}

/// `data` of `migrate`.
#[derive(Debug, Serialize)]
struct MigrateData<'a> {
    migration: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    wave: Option<u32>,
    recorded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    position: Option<u64>,
    store: &'a Path,
    /// The migration's state: after the step when it was recorded, as read when refused.
    #[serde(flatten)]
    state: &'a Waves,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    refusal: Option<&'a Refusal>,
}

impl Report<'_> {
    const fn subcommand(&self) -> &'static str {
        match self.request.step {
            Step::Start(_) => "start",
            Step::Done(_) => "done",
            Step::Contract => "contract",
        }
    }

    const fn wave(&self) -> Option<u32> {
        match self.request.step {
            Step::Start(wave) | Step::Done(wave) => Some(wave),
            Step::Contract => None,
        }
    }

    fn recorded<O: Write>(
        &self,
        state: &Waves,
        position: u64,
        out: &mut O,
    ) -> Result<Code, CliError> {
        let data = MigrateData {
            migration: self.request.migration,
            wave: self.wave(),
            recorded: true,
            position: Some(position),
            store: self.store,
            state,
            refusal: None,
        };
        Envelope::new(
            CommandName::Migrate,
            Some(self.subcommand()),
            exit_codes::MIGRATE_RECORDED,
            data,
        )
        .emit(out)
        .map_err(CliError::Output)
    }

    fn refused<O: Write>(
        &self,
        state: &Waves,
        refusal: &Refusal,
        out: &mut O,
    ) -> Result<Code, CliError> {
        let migration = self.request.migration;
        let code = match refusal {
            Refusal::WavesOpen { .. } => exit_codes::MIGRATE_WAVE_OPEN,
            _ => exit_codes::MIGRATE_REFUSED,
        };
        let finding = Finding::new(
            format!("migrate/{}:{migration}", refusal.kind()),
            refusal_message(migration, refusal),
        );
        let data = MigrateData {
            migration,
            wave: self.wave(),
            recorded: false,
            position: None,
            store: self.store,
            state,
            refusal: Some(refusal),
        };
        Envelope::new(CommandName::Migrate, Some(self.subcommand()), code, data)
            .with_findings(&[finding])
            .emit(out)
            .map_err(CliError::Output)
    }
}

/// The finding's message for `refusal`, naming the command that moves the migration on.
fn refusal_message(migration: &str, refusal: &Refusal) -> String {
    match refusal {
        Refusal::WavesOpen { waves } => format!(
            "migration {migration} has waves still open ({}); contract would remove what they \
             still read. fix: finish each with `anneal migrate done --migration {migration} \
             --wave <n>`, then contract.",
            waves
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Refusal::NoWaveDone => format!(
            "no wave of migration {migration} is done, so nothing reads the new shape yet. fix: \
             run the migrate waves first."
        ),
        Refusal::AlreadyStarted { wave } => format!(
            "wave {wave} of migration {migration} was already started. fix: number the next \
             wave anew."
        ),
        Refusal::NotOpen { wave } => format!(
            "wave {wave} of migration {migration} is not open. fix: start it with `anneal \
             migrate start --migration {migration} --wave {wave}`."
        ),
        Refusal::Contracted => format!(
            "migration {migration} is already contracted. fix: track further work under a new \
             migration id."
        ),
        Refusal::Concurrent => format!(
            "another step of migration {migration} was recorded at the same time. fix: run \
             the command again."
        ),
    }
}
