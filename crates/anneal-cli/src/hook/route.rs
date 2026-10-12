//! `anneal hook route`: the `UserPromptSubmit` hook for Clef's D3. It fails open.
//!
//! For each prompt it asks Clef, through `decide`'s own code (`decide::in_process`, then
//! `client::ask_recording`), two questions at once: the phase (`choice` over the
//! [`playbook`]'s phases) and the size (`score`, read as the playbook's nearest size level).
//! When the phase rule's action is `hint` (the top phase's probability is at least the
//! threshold, 0.60 by default), the point is in enforce mode, the phase has an entry skill and
//! the decision's row was logged, it injects one line of context naming the skill, the phase,
//! the size and its depth. Otherwise it writes nothing: Clef off, shadow mode, no answer and a
//! phase without a skill are all silent. Its own failures, an answer that could not be logged
//! among them, are one `ERROR` line as `systemMessage`. Clef is asked only while its whole
//! timeout fits in the hook's [`Budget`].
//!
//! Each answered phase decision is labelled at the session's next prompt ([`outcome`]): the
//! phase whose entry skill the model invoked in between. See `docs/hooks.md`.

mod outcome;
pub(crate) mod playbook;

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::thread;

use serde_json::json;

use crate::decide::client::{self, Ask, Asked, Decision, Outcome, Recorded};
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
use outcome::Session;
use playbook::{Playbook, Size};

/// The action of D3's phase rule that hints (`clef-thresholds.json`).
const HINT: &str = "hint";

/// The prefix of the hook's notes, so the human sees which hook wrote one.
const NOTE_PREFIX: &str = "ERROR route: ";

/// Runs the hook: reads the input from `env.stdin`, asks Clef through `transport` (each call
/// bounded by `decide`'s 2 s timeout, the two at once), and writes the output to `out`. Pass
/// `&mut out` to keep the writer.
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
    T: Transport + Sync,
    C: Clock + Sync,
{
    let budget = Budget::start(clock);
    let input = prompt::read_input(env.stdin);
    respond(input, &env.lookup, transport, (clock, budget)).write(out)
}

/// The hook's output for `input`.
fn respond(
    input: Result<Input, InputError>,
    lookup: impl Fn(&str) -> Option<OsString>,
    transport: &(impl Transport + Sync),
    (clock, budget): (&(impl Clock + Sync), Budget),
) -> Output {
    match in_process::status(Point::D3, &lookup) {
        Status::Off => return Output::default(),
        Status::Misconfigured(problem) => {
            return failed(&format!("Clef is misconfigured: {}", reason(&problem)));
        }
        Status::On => {}
    }
    let clef = match Prepared::load(&lookup) {
        Ok(clef) => clef,
        Err(error) => return failed(&reason(&error)),
    };
    let input = match input {
        Ok(input) => input,
        Err(error) => {
            return failed(&format!(
                "the hook input cannot be read: {}",
                reason(&error)
            ));
        }
    };
    let playbook = match Playbook::built_in() {
        Ok(playbook) => playbook,
        Err(error) => return failed(&reason(&error)),
    };
    let router = Router {
        clef: &clef,
        playbook: &playbook,
        transport,
        clock,
        budget,
    };
    let mut notes = Vec::new();
    let context = router.route(&input, &mut notes);
    Output {
        context,
        error: (!notes.is_empty()).then(|| format!("{NOTE_PREFIX}{}", notes.join("; "))),
    }
}

/// An output holding only the note `why`.
fn failed(why: &str) -> Output {
    Output {
        context: None,
        error: Some(format!("{NOTE_PREFIX}{why}")),
    }
}

/// What one run of the hook works with.
struct Router<'h, T, C> {
    clef: &'h Prepared,
    playbook: &'h Playbook,
    transport: &'h T,
    clock: &'h C,
    budget: Budget,
}

/// The answers to the phase and size questions, each failure already a note's text.
struct Answers<'c> {
    phase: Result<Asked<'c>, String>,
    size: Result<Asked<'c>, String>,
}

impl<T: Transport + Sync, C: Clock + Sync> Router<'_, T, C> {
    /// Labels the session's last decision, asks about `input`'s prompt and returns the hint,
    /// if one is due; what fails is appended to `notes`.
    fn route(&self, input: &Input, notes: &mut Vec<String>) -> Option<String> {
        let session = input
            .session_id
            .as_deref()
            .zip(input.transcript_path.as_deref())
            .map(|(id, transcript)| Session { id, transcript });
        if let Some(session) = session
            && let Err(error) = outcome::settle(self.clef, self.playbook, session, self.clock)
        {
            notes.push(format!("labelling the last route: {}", reason(&error)));
        }
        let state = prompt_state(input, notes)?;
        if let Err(note) = self.budget.fits_clef(self.clock) {
            notes.push(note);
            return None;
        }
        // `Ask` takes the options as `&[String]`, so the phase names are copied once per prompt.
        let options: Vec<String> = self
            .playbook
            .phases()
            .map(|p| p.name().to_owned())
            .collect();
        let phase = Ask {
            point: Point::D3,
            question: Question::Choice,
            options: &options,
            state: &state,
        };
        let size = Ask {
            question: Question::Score,
            options: &[],
            ..phase
        };
        let answers = self.ask_both(phase, size);
        let size = decided(answers.size, "size", notes)
            .and_then(|decision| self.playbook.size_for(decision.answer.probability()));
        let decision = decided(answers.phase, "phase", notes)?;
        if let (Some(session), Some(top)) = (session, decision.answer.top())
            && let Err(error) = outcome::remember(self.clef, session, &decision.stamp.id, top)
        {
            notes.push(format!(
                "keeping the route for labelling: {}",
                reason(&error)
            ));
        }
        hint(self.playbook, &decision, size)
    }

    /// Asks the phase on this thread and the size on a scoped one, so the two calls share one
    /// timeout's worth of time; the size thread is joined before this returns (ASY-10).
    fn ask_both(&self, phase: Ask<'_>, size: Ask<'_>) -> Answers<'_> {
        let Self {
            clef,
            transport,
            clock,
            ..
        } = *self;
        let setup = clef.setup();
        // The rows keep no `input`: the prompt's redacted text goes to Clef, only its hash to
        // the log.
        let phase = Recorded {
            ask: phase,
            input: None,
        };
        let size = Recorded {
            ask: size,
            input: None,
        };
        thread::scope(|scope| {
            let sizing = thread::Builder::new()
                .name("anneal-route-size".to_owned())
                .spawn_scoped(scope, || {
                    client::ask_recording(&size, &setup, transport, clock)
                });
            let phase = client::ask_recording(&phase, &setup, transport, clock)
                .map_err(|error| reason(&error));
            let size = match sizing {
                Ok(handle) => match handle.join() {
                    Ok(asked) => asked.map_err(|error| reason(&error)),
                    // The panic payload has no diagnostic value here (ERR-04).
                    Err(_panic) => Err("the size question panicked".to_owned()),
                },
                Err(error) => Err(format!("the size question could not start: {error}")),
            };
            Answers { phase, size }
        })
    }
}

/// The answered decision in `asked`, only when its row was logged: a failure to ask, or to log
/// the row, is noted (naming the `question` for the latter) and never acted on.
fn decided<'c>(
    asked: Result<Asked<'c>, String>,
    question: &str,
    notes: &mut Vec<String>,
) -> Option<Decision<'c>> {
    match asked {
        Ok(Asked {
            outcome,
            unrecorded: None,
        }) => match outcome {
            Outcome::Decided(decision) => Some(decision),
            Outcome::Off(_) | Outcome::NoAnswer(_) | Outcome::Misconfigured(_) => None,
        },
        Ok(Asked {
            unrecorded: Some(failure),
            ..
        }) => {
            notes.push(format!(
                "the {question} decision was not logged: {}",
                reason(&failure)
            ));
            None
        }
        Err(why) => {
            notes.push(why);
            None
        }
    }
}

/// The compact state for `input`'s prompt, made safe ([`safe_prompt`]); `None` when there is
/// no prompt to ask about, or (noted) when it cannot be built.
fn prompt_state(input: &Input, notes: &mut Vec<String>) -> Option<CompactState> {
    // A missing prompt asks nothing, as an empty one does.
    let prompt = input
        .prompt
        .as_deref()
        .map_or_else(String::new, safe_prompt);
    if prompt.is_empty() {
        return None;
    }
    match CompactState::parse(&json!({ "input": prompt }).to_string()) {
        Ok(state) => Some(state),
        Err(error) => {
            notes.push(reason(&error));
            None
        }
    }
}

/// The line to inject for an answered phase decision: only in enforce mode, when the rule's
/// action is [`HINT`] and the phase has an entry skill.
fn hint(playbook: &Playbook, decision: &Decision<'_>, size: Option<&Size>) -> Option<String> {
    if decision.stamp.mode != Mode::Enforce || decision.action != HINT {
        return None;
    }
    let phase = decision.answer.top()?;
    let skill = playbook.phase(phase)?.entry_skill()?;
    let detail = size.map_or_else(String::new, |size| {
        format!(", size {}, depth {}", size.name(), size.depth())
    });
    Some(format!(
        "anneal route: load the `{skill}` skill before acting (phase {phase}{detail})."
    ))
}

#[cfg(test)]
mod tests;
