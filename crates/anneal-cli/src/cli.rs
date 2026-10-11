//! The command tree and the entry point that turns arguments into an exit code.
//!
//! Every command starts as a [`Stub`] that exits 64 (not implemented). To build one out,
//! replace its `Stub` with a real `clap::Args` type (as `ledger` does with
//! [`LedgerArgs`]), keep its `after_help`, and
//! give it its own arm in the private `dispatch` function that writes an
//! [`Envelope`](crate::envelope::Envelope) and returns the code `emit` hands back.

use std::ffi::OsString;
use std::io::Write;

use clap::error::{ContextKind, ContextValue, ErrorKind};
use clap::{Args, CommandFactory, Parser, Subcommand};

use crate::command::CommandName;
use crate::error::CliError;
use crate::eval::{self, EvalArgs};
use crate::exit_codes::render::{command_help, overview_help};
use crate::exit_codes::{self, Code};
use crate::hook;
use crate::ledger::{self, LedgerArgs};
use crate::lint::{self, LintArgs};

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
    /// Mark a task done once its tests are green
    #[command(after_help = command_help(CommandName::Task))]
    Task(Stub),
    /// Approve a plan, or check that it is approved and current
    #[command(after_help = command_help(CommandName::Plan))]
    Plan(Stub),
    /// Lint skills, specs, plans and briefs
    #[command(after_help = command_help(CommandName::Lint))]
    Lint(LintArgs),
    /// Check the task graph and list its frontier
    #[command(after_help = command_help(CommandName::Dag))]
    Dag(Stub),
    /// Refuse changes that lower a quality floor
    #[command(name = "floor-guard", after_help = command_help(CommandName::FloorGuard))]
    FloorGuard(Stub),
    /// Set or validate a review verdict
    #[command(after_help = command_help(CommandName::Verdict))]
    Verdict(Stub),
    /// Watch a pull request, CI run or deploy until it settles
    #[command(after_help = command_help(CommandName::Watch))]
    Watch(Stub),
    /// Decide whether a change may skip full review
    #[command(after_help = command_help(CommandName::Ship))]
    Ship(Stub),
    /// Decide go, hold or rollback for a rollout
    #[command(after_help = command_help(CommandName::Rollout))]
    Rollout(Stub),
    /// Tell a performance change from run-to-run noise
    #[command(after_help = command_help(CommandName::Perf))]
    Perf(Stub),
    /// Build a brief from a commit range
    #[command(after_help = command_help(CommandName::Brief))]
    Brief(Stub),
    /// Ask a Clef decision point and record the answer
    #[command(after_help = command_help(CommandName::Decide))]
    Decide(Stub),
    /// Calibrate a decision point against its log
    #[command(after_help = command_help(CommandName::Calibrate))]
    Calibrate(Stub),
    /// Run evaluation suites, such as skill routing
    #[command(after_help = command_help(CommandName::Eval))]
    Eval(EvalArgs),
    /// Render the Claude Code status line
    #[command(after_help = command_help(CommandName::Statusline))]
    Statusline(Stub),
}

/// Arguments of a command that is not built yet. It accepts anything, so a caller already using
/// the future syntax (`anneal task done ...`) gets 64, not a usage error.
#[derive(Debug, Args)]
pub struct Stub {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    args: Vec<OsString>,
}

impl Command {
    /// Which top-level command this is.
    #[must_use]
    pub const fn name(&self) -> CommandName {
        match self {
            Self::Ledger(_) => CommandName::Ledger,
            Self::Task(_) => CommandName::Task,
            Self::Plan(_) => CommandName::Plan,
            Self::Lint(_) => CommandName::Lint,
            Self::Dag(_) => CommandName::Dag,
            Self::FloorGuard(_) => CommandName::FloorGuard,
            Self::Verdict(_) => CommandName::Verdict,
            Self::Watch(_) => CommandName::Watch,
            Self::Ship(_) => CommandName::Ship,
            Self::Rollout(_) => CommandName::Rollout,
            Self::Perf(_) => CommandName::Perf,
            Self::Brief(_) => CommandName::Brief,
            Self::Decide(_) => CommandName::Decide,
            Self::Calibrate(_) => CommandName::Calibrate,
            Self::Eval(_) => CommandName::Eval,
            Self::Statusline(_) => CommandName::Statusline,
        }
    }
}

/// Runs `anneal` with `args` (including the program name), writing results to `out` and
/// errors to `err`, and returns the exit code. `out` is flushed before returning.
///
/// On success `err` stays empty; on failure it gets exactly one `ERROR <reason>` line and the
/// code is the error's documented code, never 0.
pub fn run<I, T, O, E>(args: I, mut out: O, mut err: E) -> Code
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
    O: Write,
    E: Write,
{
    let result = execute(args, &mut out)
        .and_then(|code| out.flush().map(|()| code).map_err(CliError::Output));
    match result {
        Ok(code) => code,
        Err(error) => match hook::write_error(&mut err, &error) {
            Ok(()) => error.code(),
            // stderr is closed too: the exit code is the only report left.
            Err(_unreportable) => exit_codes::OUTPUT,
        },
    }
}

fn execute<I, T, O>(args: I, out: &mut O) -> Result<Code, CliError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
    O: Write,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(error) => return clap_outcome(&error, args.last(), out),
    };
    match cli.command {
        None => {
            let help = Cli::command().render_help();
            write!(out, "{help}").map_err(CliError::Output)?;
            Ok(exit_codes::OK)
        }
        Some(command) => dispatch(&command, out),
    }
}

/// Runs one command. Each real command gets its own arm; stubs share the last one.
///
/// The process's working directory and environment are read here, at the edge, and passed
/// in; only the commands that use them read them, so `--help` and the stubs still work in a
/// directory that no longer exists.
fn dispatch<O: Write>(command: &Command, out: &mut O) -> Result<Code, CliError> {
    match command {
        Command::Ledger(args) => {
            let cwd = std::env::current_dir().map_err(CliError::CurrentDir)?;
            ledger::run(args, &cwd, |var| std::env::var_os(var), out)
        }
        Command::Lint(args) => lint::run(args, out),
        Command::Eval(args) => eval::run(args, out),
        Command::Task(_)
        | Command::Plan(_)
        | Command::Dag(_)
        | Command::FloorGuard(_)
        | Command::Verdict(_)
        | Command::Watch(_)
        | Command::Ship(_)
        | Command::Rollout(_)
        | Command::Perf(_)
        | Command::Brief(_)
        | Command::Decide(_)
        | Command::Calibrate(_)
        | Command::Statusline(_) => Err(CliError::NotImplemented {
            command: command.name(),
        }),
    }
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
