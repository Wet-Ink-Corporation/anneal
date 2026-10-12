//! The command tree and the entry point that turns arguments into an exit code.
//!
//! Every command has a real `clap::Args` type (as `ledger` has [`LedgerArgs`]), its
//! `after_help`, and its own arm in the private `dispatch` function that writes an
//! [`Envelope`](crate::envelope::Envelope) and returns the code `emit` hands back.

use std::ffi::OsString;
use std::io::Write;

use clap::error::{ContextKind, ContextValue, ErrorKind};
use clap::{CommandFactory, Parser, Subcommand};

use crate::brief::{self, BriefArgs};
use crate::calibrate::{self, CalibrateArgs, CalibrateEnv};
use crate::command::CommandName;
use crate::dag::{self, DagArgs};
use crate::decide::clock::SystemClock;
use crate::decide::transport::http::UreqTransport;
use crate::decide::{self, DecideArgs, DecideEnv};
use crate::error::CliError;
use crate::eval::{self, EvalArgs};
use crate::exit_codes::render::{command_help, overview_help};
use crate::exit_codes::{self, Code};
use crate::floor_guard::{self, FloorGuardArgs};
use crate::grill::{self, GrillArgs};
use crate::hook::{self, HookArgs, HookEnv};
use crate::ledger::{self, LedgerArgs};
use crate::lint::{self, LintArgs};
use crate::migrate::{self, MigrateArgs};
use crate::mode::{self, ModeArgs};
use crate::orchestrate::{self, OrchestrateArgs};
use crate::perf::runner::ProcessRunner;
use crate::perf::{self, PerfArgs};
use crate::plan::{self, PlanArgs};
use crate::repro::{self, ReproArgs};
use crate::rollout::{self, RolloutArgs};
use crate::round::{self, RoundArgs};
use crate::ship::{self, ShipArgs};
use crate::statusline::{self, StatuslineArgs};
use crate::task::{self, TaskArgs};
use crate::verdict::{self, VerdictArgs};
use crate::watch::{self, WatchArgs};
use crate::weighin;

/// The flags clap answers by printing instead of running a command.
const HELP_AND_VERSION: [&str; 4] = ["-h", "--help", "-V", "--version"];

/// `anneal`: the parsed command line.
#[derive(Debug, Parser)]
#[command(
    name = "anneal",
    version,
    about = "anneal: enforced SDLC gates for Claude Code",
    disable_help_subcommand = true,
    after_help = overview_help()
)]
pub struct Cli {
    /// The command to run; none prints this help.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// The top-level commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Append to, query and tail the event ledger
    #[command(after_help = command_help(CommandName::Ledger))]
    Ledger(LedgerArgs),
    /// List a design tree's open frontier, classify a question with Clef D7, or record the
    /// human's confirmation of the tree
    #[command(after_help = command_help(CommandName::Grill))]
    Grill(GrillArgs),
    /// Mark a task done once its tests are green
    #[command(after_help = command_help(CommandName::Task))]
    Task(TaskArgs),
    /// Approve a plan, or check that it is approved and current
    #[command(after_help = command_help(CommandName::Plan))]
    Plan(PlanArgs),
    /// Enter build or diagnose mode, whose preconditions gate source edits, or leave it
    #[command(after_help = command_help(CommandName::Mode))]
    Mode(ModeArgs),
    /// Record a red reproduction for diagnose mode, or list `[DEBUG-<id>]` leftovers
    #[command(after_help = command_help(CommandName::Repro))]
    Repro(ReproArgs),
    /// Lint skills, specs, plans and briefs
    #[command(after_help = command_help(CommandName::Lint))]
    Lint(LintArgs),
    /// Check the task graph and list its frontier
    #[command(after_help = command_help(CommandName::Dag))]
    Dag(DagArgs),
    /// Refuse changes that lower a quality floor
    #[command(name = "floor-guard", after_help = command_help(CommandName::FloorGuard))]
    FloorGuard(FloorGuardArgs),
    /// Set a review verdict for a pull request's head, or check that it has a fresh one
    #[command(after_help = command_help(CommandName::Verdict))]
    Verdict(VerdictArgs),
    /// Watch a pull request, CI run or deploy until it settles
    #[command(after_help = command_help(CommandName::Watch))]
    Watch(WatchArgs),
    /// Decide whether a change may skip full review
    #[command(after_help = command_help(CommandName::Ship))]
    Ship(ShipArgs),
    /// Decide go, hold or rollback for a rollout
    #[command(after_help = command_help(CommandName::Rollout))]
    Rollout(RolloutArgs),
    /// Tell a performance change from run-to-run noise
    #[command(after_help = command_help(CommandName::Perf))]
    Perf(PerfArgs),
    /// Track an expand, migrate, contract migration wave by wave
    #[command(after_help = command_help(CommandName::Migrate))]
    Migrate(MigrateArgs),
    /// Build a ticket's brief and review package from a range rooted at a base
    #[command(after_help = command_help(CommandName::Brief))]
    Brief(BriefArgs),
    /// Start a ticket's next fix-loop round, or print its round count and what comes next
    #[command(after_help = command_help(CommandName::Round))]
    Round(RoundArgs),
    /// Gate fan-out on the frontier, park a one-way call in weigh-in, or record a ruling
    #[command(after_help = command_help(CommandName::Orchestrate))]
    Orchestrate(OrchestrateArgs),
    /// Ask a Clef decision point and record the answer
    #[command(after_help = command_help(CommandName::Decide))]
    Decide(DecideArgs),
    /// Score each Clef decision point against its labelled outcomes and propose thresholds
    #[command(after_help = command_help(CommandName::Calibrate))]
    Calibrate(CalibrateArgs),
    /// Run evaluation suites, such as skill routing
    #[command(after_help = command_help(CommandName::Eval))]
    Eval(EvalArgs),
    /// Render the Claude Code status line and save the context usage for the context monitor
    /// (fails open, like a hook)
    #[command(after_help = command_help(CommandName::Statusline))]
    Statusline(StatuslineArgs),
    /// Run a plugin hook (hooks.json calls these; they fail open unless passed --fail-closed,
    /// and the guard and preconditions hooks always fail closed)
    #[command(after_help = command_help(CommandName::Hook))]
    Hook(HookArgs),
}

/// What a failure after parsing does to the exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnFailure {
    /// The error's own code (64, 70, 74, ...).
    Exit,
    /// Still exit 0: a hook fails open. The error is still written as one `ERROR` line.
    FailOpen,
    /// Exit 2, which Claude Code reads as "block": a guard fails closed. The error is still
    /// written as one `ERROR` line.
    FailClosed,
}

impl Command {
    /// Which top-level command this is.
    #[must_use]
    pub const fn name(&self) -> CommandName {
        match self {
            Self::Ledger(_) => CommandName::Ledger,
            Self::Grill(_) => CommandName::Grill,
            Self::Task(_) => CommandName::Task,
            Self::Plan(_) => CommandName::Plan,
            Self::Mode(_) => CommandName::Mode,
            Self::Repro(_) => CommandName::Repro,
            Self::Lint(_) => CommandName::Lint,
            Self::Dag(_) => CommandName::Dag,
            Self::FloorGuard(_) => CommandName::FloorGuard,
            Self::Verdict(_) => CommandName::Verdict,
            Self::Watch(_) => CommandName::Watch,
            Self::Ship(_) => CommandName::Ship,
            Self::Rollout(_) => CommandName::Rollout,
            Self::Perf(_) => CommandName::Perf,
            Self::Migrate(_) => CommandName::Migrate,
            Self::Brief(_) => CommandName::Brief,
            Self::Round(_) => CommandName::Round,
            Self::Orchestrate(_) => CommandName::Orchestrate,
            Self::Decide(_) => CommandName::Decide,
            Self::Calibrate(_) => CommandName::Calibrate,
            Self::Eval(_) => CommandName::Eval,
            Self::Statusline(_) => CommandName::Statusline,
            Self::Hook(_) => CommandName::Hook,
        }
    }

    /// Hooks fail open and guards closed; every other command exits with its error's code.
    /// Exhaustive, so a new command has to choose.
    const fn on_failure(&self) -> OnFailure {
        match self {
            Self::Hook(args) if args.fails_closed() => OnFailure::FailClosed,
            Self::Hook(_) | Self::Statusline(_) => OnFailure::FailOpen,
            Self::Ledger(_)
            | Self::Grill(_)
            | Self::Task(_)
            | Self::Plan(_)
            | Self::Mode(_)
            | Self::Repro(_)
            | Self::Lint(_)
            | Self::Dag(_)
            | Self::FloorGuard(_)
            | Self::Verdict(_)
            | Self::Watch(_)
            | Self::Ship(_)
            | Self::Rollout(_)
            | Self::Perf(_)
            | Self::Migrate(_)
            | Self::Brief(_)
            | Self::Round(_)
            | Self::Orchestrate(_)
            | Self::Decide(_)
            | Self::Calibrate(_)
            | Self::Eval(_) => OnFailure::Exit,
        }
    }
}

/// Runs `anneal` with `args` (including the program name), writing results to `out` and
/// errors to `err`, and returns the exit code. `out` is flushed before returning.
///
/// On success `err` stays empty, except for `anneal calibrate`'s short human summary, written
/// after its envelope; on failure it gets exactly one `ERROR <reason>` line and the code is the
/// error's documented code, never 0. The exceptions are `anneal hook` and `anneal statusline`,
/// which fail open: after the `ERROR` line they still exit 0 (only usage errors exit 2). A guard
/// (`hook guard`, or a hook passed `--fail-closed`) fails closed instead: it exits 2.
pub fn run<I, T, O, E>(args: I, mut out: O, mut err: E) -> Code
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
    O: Write,
    E: Write,
{
    let (result, on_failure) = execute(args, &mut out, &mut err);
    let result = result.and_then(|code| out.flush().map(|()| code).map_err(CliError::Output));
    let error = match result {
        Ok(code) => return code,
        Err(error) => error,
    };
    let reported = hook::write_error(&mut err, &error);
    match (on_failure, reported) {
        // A hook fails open whether or not stderr took the line.
        (OnFailure::FailOpen, _) => exit_codes::HOOK_DONE,
        // A guard blocks whether or not stderr took the line.
        (OnFailure::FailClosed, _) => exit_codes::HOOK_BLOCKED,
        (OnFailure::Exit, Ok(())) => error.code(),
        // stderr is closed too: the exit code is the only report left.
        (OnFailure::Exit, Err(_unreportable)) => exit_codes::OUTPUT,
    }
}

/// Parses `args` and runs the command, returning its result and what a failure of it does to
/// the exit code. A usage error always exits with its code.
fn execute<I, T, O, E>(args: I, out: &mut O, err: &mut E) -> (Result<Code, CliError>, OnFailure)
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
    O: Write,
    E: Write,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    match Cli::try_parse_from(&args) {
        Err(error) => (clap_outcome(&error, args.last(), out), OnFailure::Exit),
        Ok(Cli { command: None }) => {
            let help = Cli::command().render_help();
            let written = write!(out, "{help}").map_err(CliError::Output);
            (written.map(|()| exit_codes::OK), OnFailure::Exit)
        }
        Ok(Cli {
            command: Some(command),
        }) => (dispatch(&command, out, err), command.on_failure()),
    }
}

/// Runs one command. Each real command gets its own arm; stubs share the last one.
///
/// The process's working directory and environment are read here, at the edge, and passed
/// in; only the commands that use them read them, so `--help` and the stubs still work in a
/// directory that no longer exists. `err` is for a command's own human output on success
/// (`calibrate`'s summary); errors are written by [`run`].
fn dispatch<O: Write, E: Write>(
    command: &Command,
    out: &mut O,
    err: &mut E,
) -> Result<Code, CliError> {
    match command {
        Command::Ledger(args) => {
            let cwd = std::env::current_dir().map_err(CliError::CurrentDir)?;
            ledger::run(args, &cwd, |var| std::env::var_os(var), out)
        }
        Command::Grill(args) => run_grill(args, out),
        Command::Plan(args) => {
            let cwd = std::env::current_dir().map_err(CliError::CurrentDir)?;
            plan::run(args, &cwd, |var| std::env::var_os(var), out)
        }
        Command::Mode(args) => {
            let cwd = std::env::current_dir().map_err(CliError::CurrentDir)?;
            mode::run(args, &cwd, |var| std::env::var_os(var), out)
        }
        Command::Repro(args) => {
            let cwd = std::env::current_dir().map_err(CliError::CurrentDir)?;
            repro::run(args, &cwd, |var| std::env::var_os(var), out)
        }
        Command::Task(args) => {
            let cwd = std::env::current_dir().map_err(CliError::CurrentDir)?;
            task::run(args, &cwd, |var| std::env::var_os(var), out)
        }
        Command::Lint(args) => lint::run(args, out),
        Command::Dag(args) => {
            let cwd = std::env::current_dir().map_err(CliError::CurrentDir)?;
            dag::run(args, &cwd, out)
        }
        Command::FloorGuard(args) => {
            let cwd = std::env::current_dir().map_err(CliError::CurrentDir)?;
            floor_guard::run(args, &cwd, out)
        }
        Command::Verdict(args) => {
            let cwd = std::env::current_dir().map_err(CliError::CurrentDir)?;
            verdict::run(args, &cwd, |var| std::env::var_os(var), out)
        }
        Command::Eval(args) => eval::run(args, out),
        Command::Decide(args) => run_decide(args, out),
        Command::Calibrate(args) => run_calibrate(args, out, err),
        Command::Hook(args) => run_hook(args, out),
        Command::Ship(args) => {
            let cwd = std::env::current_dir().map_err(CliError::CurrentDir)?;
            ship::run(args, &cwd, out)
        }
        Command::Watch(args) => run_watch(args, out),
        Command::Statusline(StatuslineArgs) => run_statusline(out, err),
        Command::Rollout(args) => rollout::run(args, out),
        Command::Perf(args) => perf::run(args, &mut ProcessRunner, out),
        Command::Migrate(args) => run_migrate(args, out),
        Command::Brief(args) => run_brief(args, out),
        Command::Round(args) => run_round(args, out),
        Command::Orchestrate(args) => run_orchestrate(args, out),
    }
}

/// `anneal orchestrate` from the process's working directory, with its environment for the
/// ledger and `weighin`, which runs as a child process.
fn run_orchestrate<O: Write>(args: &OrchestrateArgs, out: &mut O) -> Result<Code, CliError> {
    let cwd = std::env::current_dir().map_err(CliError::CurrentDir)?;
    let ports = orchestrate::Ports {
        cwd: &cwd,
        lookup: |var: &str| std::env::var_os(var),
        runner: &weighin::ProcessRunner,
    };
    orchestrate::run(args, &ports, out)
}

/// `anneal hook` with the process's stdin, working directory and environment.
fn run_hook<O: Write>(args: &HookArgs, out: &mut O) -> Result<Code, CliError> {
    let env = HookEnv {
        stdin: std::io::stdin().lock(),
        current_dir: std::env::current_dir,
        lookup: |var: &str| std::env::var_os(var),
    };
    hook::run(args, env, out)
        .map(|()| exit_codes::HOOK_DONE)
        .map_err(CliError::Output)
}

/// `anneal migrate` from the process's working directory, with its environment for the ledger.
fn run_migrate<O: Write>(args: &MigrateArgs, out: &mut O) -> Result<Code, CliError> {
    let cwd = std::env::current_dir().map_err(CliError::CurrentDir)?;
    migrate::run(args, &cwd, |var| std::env::var_os(var), out)
}

/// `anneal decide` with the process's environment, the system clock and the transport.
fn run_decide<O: Write>(args: &DecideArgs, out: &mut O) -> Result<Code, CliError> {
    let cwd = std::env::current_dir().map_err(CliError::CurrentDir)?;
    let env = DecideEnv {
        cwd: &cwd,
        lookup: |var: &str| std::env::var_os(var),
        transport: &UreqTransport::new(),
        clock: &SystemClock::new(),
    };
    decide::run(args, &env, out)
}

/// `anneal grill` with the process's working directory and environment, the system clock and
/// the transport `grill classify` asks Clef D7 through.
fn run_grill<O: Write>(args: &GrillArgs, out: &mut O) -> Result<Code, CliError> {
    let cwd = std::env::current_dir().map_err(CliError::CurrentDir)?;
    let env = DecideEnv {
        cwd: &cwd,
        lookup: |var: &str| std::env::var_os(var),
        transport: &UreqTransport::new(),
        clock: &SystemClock::new(),
    };
    grill::run(args, &env, out)
}

/// `anneal watch` from the process's working directory, with its environment for the merge
/// gate's ledger.
fn run_watch<O: Write>(args: &WatchArgs, out: &mut O) -> Result<Code, CliError> {
    let cwd = std::env::current_dir().map_err(CliError::CurrentDir)?;
    watch::run(args, &cwd, |var| std::env::var_os(var), out)
}

/// `anneal statusline` with the process's stdin and environment.
fn run_statusline<O: Write, E: Write>(out: &mut O, err: &mut E) -> Result<Code, CliError> {
    statusline::run(
        std::io::stdin().lock(),
        |var| std::env::var_os(var),
        out,
        err,
    )
}

/// `anneal round` from the process's working directory, with its environment for the ledger.
fn run_round<O: Write>(args: &RoundArgs, out: &mut O) -> Result<Code, CliError> {
    let cwd = std::env::current_dir().map_err(CliError::CurrentDir)?;
    round::run(args, &cwd, |var| std::env::var_os(var), out)
}

/// `anneal brief` from the process's working directory.
fn run_brief<O: Write>(args: &BriefArgs, out: &mut O) -> Result<Code, CliError> {
    let cwd = std::env::current_dir().map_err(CliError::CurrentDir)?;
    brief::run(args, &cwd, out)
}

/// `anneal calibrate` with the process's working directory and environment.
fn run_calibrate<O: Write, E: Write>(
    args: &CalibrateArgs,
    out: &mut O,
    err: &mut E,
) -> Result<Code, CliError> {
    let cwd = std::env::current_dir().map_err(CliError::CurrentDir)?;
    let env = CalibrateEnv {
        cwd: &cwd,
        lookup: |var: &str| std::env::var_os(var),
    };
    calibrate::run(args, &env, out, err)
}

/// `--help` and `--version` go to stdout and exit 0 when they are the last argument; anything
/// after them, and anything else clap rejects, is a usage error, so it never exits 0.
fn clap_outcome<O: Write>(
    error: &clap::Error,
    last: Option<&OsString>,
    out: &mut O,
) -> Result<Code, CliError> {
    let flag_is_last = last.is_some_and(|arg| HELP_AND_VERSION.iter().any(|flag| arg == flag));
    match error.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion if !flag_is_last => {
            Err(CliError::Usage {
                reason: "unexpected argument after --help or --version".to_owned(),
            })
        }
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
            write!(out, "{}", error.render()).map_err(CliError::Output)?;
            Ok(exit_codes::OK)
        }
        _ => Err(CliError::Usage {
            reason: usage_reason(error),
        }),
    }
}

/// One line: `unknown argument: <arg>` for an unknown command or flag, else clap's first line.
fn usage_reason(error: &clap::Error) -> String {
    let unknown = match error.kind() {
        ErrorKind::InvalidSubcommand => error.get(ContextKind::InvalidSubcommand),
        ErrorKind::UnknownArgument => error.get(ContextKind::InvalidArg),
        _ => None,
    };
    if let Some(ContextValue::String(arg)) = unknown {
        return format!("unknown argument: {arg}");
    }
    let rendered = error.render().to_string();
    let first = rendered.lines().next().unwrap_or("invalid arguments");
    first.strip_prefix("error: ").unwrap_or(first).to_owned()
}

#[cfg(test)]
mod tests;
