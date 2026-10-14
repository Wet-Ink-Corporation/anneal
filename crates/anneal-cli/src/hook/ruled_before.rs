//! `anneal hook ruled-before`: the `UserPromptSubmit` hook for Clef's D6. It fails open.
//!
//! For each prompt it gathers the repository's rulings ([`precedents`]: the `.out-of-scope/`
//! store, accepted weigh-in decision atoms, and the items `weighin open --all --json` lists as
//! decided whose atoms are not written yet) and asks Clef, through `decide`'s own code
//! (`decide::in_process`, then `client::ask_recording`), which of them the prompt would
//! re-open: a `choice` over their ids plus `none`. When the rule's action is `surface` (the top
//! option's probability is at least the threshold, 0.70 by default), the top option is a
//! ruling, the point is in enforce mode and the decision's row was logged, it injects one line
//! naming the ruling and saying not to re-open it. Otherwise it writes nothing: Clef off, no
//! rulings, shadow mode, `none` and no answer are all silent. Its own failures (an invalid
//! ruling, or an answer that could not be logged, among them) are one `ERROR` line as
//! `systemMessage`. Within the hook's [`Budget`], git gets at most [`GIT_TIMEOUT`] and weigh-in
//! at most [`WEIGH_IN_TIMEOUT`], neither the time Clef's own timeout needs, and Clef is asked
//! only while that timeout fits. With no `weighin` (no `$WEIGHIN_BIN`, none on `PATH`) weigh-in
//! is not asked; one that fails leaves the other rulings offered and adds one `ERROR` note. See
//! `docs/hooks.md`.

mod precedents;

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use crate::decide::client::{self, Ask, Outcome, Recorded};
use crate::decide::clock::Clock;
use crate::decide::config::Mode;
use crate::decide::in_process::{self, Prepared, Status};
use crate::decide::point::{Point, Question};
use crate::decide::redact::safe_prompt;
use crate::decide::state::CompactState;
use crate::decide::transport::Transport;
use crate::error_chain::reason;
use crate::hook::HookEnv;
use crate::hook::prompt::{self, Budget, Input, InputError, Output};
use crate::paths::{self, PathError};
use crate::weighin::{self, BridgeError, Decided, ProcessRunner, Runner};
use precedents::{NONE, Precedent};

/// The action of D6's rule that surfaces a ruling (`clef-thresholds.json`).
const SURFACE: &str = "surface";

/// The prefix of the hook's notes, so the human sees which hook wrote one.
const NOTE_PREFIX: &str = "ERROR ruled-before: ";

/// Most time git may take to find the repository root, when the budget has it to spare.
const GIT_TIMEOUT: Duration = Duration::from_secs(1);

/// Most time `weighin open --all --json` may take, when the budget has it to spare.
const WEIGH_IN_TIMEOUT: Duration = Duration::from_secs(1);

/// Runs the hook: reads the input from `env.stdin`, gathers the rulings of the repository
/// around the input's `cwd` (or `env.current_dir`), asks Clef through `transport` and writes
/// the output to `out`. Pass `&mut out` to keep the writer.
///
/// # Errors
/// Only the I/O error from writing `out`; every other failure is a note in the output.
pub(crate) fn run<R, D, L, W, T, C>(
    env: HookEnv<R, D, L>,
    transport: &T,
    clock: &C,
    out: W,
) -> io::Result<()>
where
    R: Read,
    D: FnOnce() -> io::Result<PathBuf>,
    L: Fn(&str) -> Option<OsString>,
    W: Write,
    T: Transport,
    C: Clock,
{
    let ports = Ports {
        transport,
        clock,
        runner: &ProcessRunner,
    };
    run_with(env, &ports, out)
}

/// What the hook reaches Clef, time and weigh-in through.
pub(crate) struct Ports<'a, T, C, U> {
    /// Clef's transport.
    pub(crate) transport: &'a T,
    /// The clock the budget and the decision rows read.
    pub(crate) clock: &'a C,
    /// Runs `weighin open --all --json`.
    pub(crate) runner: &'a U,
}

/// [`run`], reaching Clef, time and weigh-in through `ports`.
///
/// # Errors
/// Only the I/O error from writing `out`.
pub(crate) fn run_with<R, D, L, W, T, C, U>(
    env: HookEnv<R, D, L>,
    ports: &Ports<'_, T, C, U>,
    out: W,
) -> io::Result<()>
where
    R: Read,
    D: FnOnce() -> io::Result<PathBuf>,
    L: Fn(&str) -> Option<OsString>,
    W: Write,
    T: Transport,
    C: Clock,
    U: Runner,
{
    let budget = Budget::start(ports.clock);
    let input = prompt::read_input(env.stdin);
    match in_process::status(Point::D6, &env.lookup) {
        Status::Off => return Output::default().write(out),
        Status::Misconfigured(problem) => {
            return failed(&format!("Clef is misconfigured: {}", reason(&problem))).write(out);
        }
        Status::On => {}
    }
    let clef = match Prepared::load(&env.lookup) {
        Ok(clef) => clef,
        Err(error) => return failed(&reason(&error)).write(out),
    };
    let timed = Timed { ports, budget };
    let output = match input {
        Ok(input) => surface(&clef, &input, (env.current_dir, &env.lookup), &timed)
            .unwrap_or_else(|why| failed(&why)),
        Err(error) => failed(&input_note(&error)),
    };
    output.write(out)
}

/// The note for input that cannot be used.
fn input_note(error: &InputError) -> String {
    format!("the hook input cannot be read: {}", reason(error))
}

/// An output holding only the note `why`.
fn failed(why: &str) -> Output {
    Output {
        context: None,
        error: Some(format!("{NOTE_PREFIX}{why}")),
    }
}

/// The hook's ports and the budget measured from its start.
struct Timed<'a, T, C, U> {
    ports: &'a Ports<'a, T, C, U>,
    budget: Budget,
}

impl<T, C: Clock, U> Timed<'_, T, C, U> {
    /// What the budget can spare beside Clef's whole timeout, at most `most`.
    fn spare(&self, most: Duration) -> Duration {
        self.budget
            .left(self.ports.clock)
            .saturating_sub(client::TIMEOUT)
            .min(most)
    }
}

/// Asks about `input`'s prompt and returns the output: the line to inject, if one is due, and
/// the note when weigh-in's decided items could not be read.
///
/// # Errors
/// The note to write, when the repository, its rulings or the log cannot be read, when the
/// answer's row could not be logged, or when the budget leaves no time for Clef.
fn surface<T: Transport, C: Clock, U: Runner>(
    clef: &Prepared,
    input: &Input,
    (current_dir, lookup): (
        impl FnOnce() -> io::Result<PathBuf>,
        &impl Fn(&str) -> Option<OsString>,
    ),
    timed: &Timed<'_, T, C, U>,
) -> Result<Output, String> {
    let prompt = input
        .prompt
        .as_deref()
        .map_or_else(String::new, safe_prompt);
    if prompt.is_empty() {
        return Ok(Output::default());
    }
    let current;
    let cwd = if let Some(cwd) = &input.cwd {
        cwd
    } else {
        current =
            current_dir().map_err(|error| format!("reading the working directory: {error}"))?;
        &current
    };
    // Git gets what the budget can spare beside Clef's whole timeout, at most `GIT_TIMEOUT`;
    // with none to spare, nothing is started.
    timed.budget.fits_clef(timed.ports.clock)?;
    let root = repository_root(cwd, timed.spare(GIT_TIMEOUT))?;
    // weigh-in gets what is left beside Clef's whole timeout, at most `WEIGH_IN_TIMEOUT`.
    let spare = timed.spare(WEIGH_IN_TIMEOUT);
    let (decided, weigh_in_note) = match decided_items(&root, lookup, timed.ports.runner, spare) {
        Ok(decided) => (decided, None),
        Err(why) => (Vec::new(), Some(format!("{NOTE_PREFIX}{why}"))),
    };
    let found = precedents::gather(&root, &decided).map_err(|error| reason(&error))?;
    let context = if found.is_empty() {
        None
    } else {
        ask(clef, &prompt, &found, timed)?
    };
    Ok(Output {
        context,
        error: weigh_in_note,
    })
}

/// Asks Clef which ruling of `found` the redacted `prompt` would re-open, returning the line to
/// inject when one is due.
///
/// # Errors
/// As [`surface`].
fn ask<T: Transport, C: Clock, U>(
    clef: &Prepared,
    prompt: &str,
    found: &[Precedent],
    timed: &Timed<'_, T, C, U>,
) -> Result<Option<String>, String> {
    let ports = timed.ports;
    let state = CompactState::parse(&state_for(prompt, found).to_string())
        .map_err(|error| reason(&error))?;
    // `Ask` takes the options as `&[String]`: the ids are copied once, with `none` last.
    let options: Vec<String> = found
        .iter()
        .map(|precedent| precedent.id().to_owned())
        .chain([NONE.to_owned()])
        .collect();
    let ask = Ask {
        point: Point::D6,
        question: Question::Choice,
        options: &options,
        state: &state,
    };
    timed.budget.fits_clef(ports.clock)?;
    // The row keeps no `input`: the prompt's redacted text goes to Clef, only its hash to the
    // log.
    let question = Recorded { ask, input: None };
    let asked = client::ask_recording(&question, &clef.setup(), ports.transport, ports.clock)
        .map_err(|error| reason(&error))?;
    if let Some(failure) = &asked.unrecorded {
        // Never act on an answer that is not in the log.
        return Err(format!("the decision was not logged: {}", reason(failure)));
    }
    match asked.outcome {
        Outcome::Decided(decision) => {
            let enforced = decision.stamp.mode == Mode::Enforce && decision.action == SURFACE;
            let ruling = decision.answer.top().and_then(|top| find(found, top));
            Ok(ruling.filter(|_| enforced).map(line))
        }
        Outcome::Off(_) | Outcome::NoAnswer(_) | Outcome::Misconfigured(_) => Ok(None),
    }
}

/// The items weigh-in has decided, from `weighin open --all --json` in `root` within `time`;
/// none when there is no `weighin` to ask.
///
/// # Errors
/// The note (without its prefix) when `weighin` was named but could not be run or read, or no
/// time is left for it.
fn decided_items(
    root: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
    runner: &impl Runner,
    time: Duration,
) -> Result<Vec<Decided>, String> {
    let not_read = |why: &str| format!("weigh-in's decided items were not read: {why}");
    let bin = match weighin::locate(lookup) {
        Ok(located) => located.path,
        Err(BridgeError::NotFound) => return Ok(Vec::new()),
        Err(error) => return Err(not_read(&reason(&error))),
    };
    if time.is_zero() {
        return Err(not_read("no time was left in the hook's budget"));
    }
    weighin::decided_items(runner, &bin, root, time).map_err(|error| not_read(&reason(&error)))
}

/// The compact state's JSON for `prompt` (already safe) and the rulings `found`.
fn state_for(prompt: &str, found: &[Precedent]) -> Value {
    json!({
        "input": prompt,
        "rulings": found
            .iter()
            .map(|p| json!({"id": p.id(), "summary": p.summary()}))
            .collect::<Vec<_>>(),
    })
}

/// The root of the git worktree around `cwd`, or `cwd` itself outside a repository; git gets
/// `timeout`.
fn repository_root(cwd: &Path, timeout: Duration) -> Result<PathBuf, String> {
    match paths::worktree_root_within(cwd, timeout) {
        Ok(root) => Ok(PathBuf::from(root)),
        Err(PathError::NotARepository { .. }) => Ok(cwd.to_path_buf()),
        Err(error) => Err(format!("finding the repository: {}", reason(&error))),
    }
}

/// The precedent with option id `id` (never `none`).
fn find<'p>(found: &'p [Precedent], id: &str) -> Option<&'p Precedent> {
    found.iter().find(|precedent| precedent.id() == id)
}

/// The line that surfaces `ruling`: the fixed part first, then the ruling.
fn line(ruling: &Precedent) -> String {
    format!(
        "anneal ruled-before: this was ruled on already; do not re-open it unless the human \
         asks to: {} ({}).",
        ruling.id(),
        ruling.summary()
    )
}

#[cfg(test)]
mod tests;
