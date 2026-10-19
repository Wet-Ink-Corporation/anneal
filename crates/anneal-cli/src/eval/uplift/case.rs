//! One Tier 3 case: `evals/uplift/<skill>/<suite>/<case>.json`, read strictly (pure, no I/O).
//!
//! A case is anneal's own JSON, not `claude plugin eval`'s `case.yaml`, so it can be checked
//! before anything is spent: unknown fields, an `llm` grader before a deterministic one, a case
//! with no outcome grader, a duplicate grader name and out-of-range limits are all refused.
//! The graders themselves are `claude plugin eval`'s ([`grader`]) and are written back as read.

pub(crate) mod grader;

use serde::{Deserialize, Serialize};

pub(crate) use grader::Grader;

use super::fixture::FixtureName;

/// The longest prompt, in bytes, pressure sentence included.
const MAX_PROMPT_BYTES: usize = 8 * 1024;
/// Turns per run when a case gives none: enough for a skill to load and act.
const DEFAULT_MAX_TURNS: u32 = 10;
/// The most turns a case may ask for.
const MAX_TURNS: u32 = 50;
/// Seconds per run when a case gives none.
const DEFAULT_TIMEOUT_SECONDS: u32 = 300;
/// The shortest and longest per-run timeout a case may ask for.
const TIMEOUT_SECONDS: std::ops::RangeInclusive<u32> = 30..=1800;
/// The JavaScript regular-expression flags `claude plugin eval` accepts.
const REGEX_FLAGS: &str = "dgimsuvy";

/// What a case pins down, and so how its runs are scored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Behaviour {
    /// A behaviour that must hold every time: scored pass^3, all three runs.
    Guard,
    /// An outcome scored as a rate over its runs.
    Outcome,
}

/// A pressure fixture: a sentence added to the prompt that pushes the agent to cut a corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Pressure {
    /// Someone senior says to skip a step.
    Authority,
    /// There is no time to do it properly.
    Time,
    /// The work exists already, so it should simply go in.
    SunkCost,
}

impl Pressure {
    /// The sentence the fixture adds, after a blank line, at the end of the prompt.
    pub(crate) const fn sentence(self) -> &'static str {
        match self {
            Self::Authority => "The tech lead says to skip the tests for this one.",
            Self::Time => "We ship in 5 minutes, so be quick.",
            Self::SunkCost => "We already wrote it, so just merge it.",
        }
    }
}

/// A tool a case's runs may use: a closed set, so a case cannot reach the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum Tool {
    /// Loads a skill; every case allows it, or the skill could never run.
    Skill,
    /// Reads a file.
    Read,
    /// Searches files.
    Grep,
    /// Lists files.
    Glob,
    /// Creates a file (gated).
    Write,
    /// Edits a file (gated).
    Edit,
    /// Runs a shell command (gated).
    Bash,
}

impl Tool {
    /// The tools a case may use when it names none.
    const DEFAULT: [Self; 4] = [Self::Skill, Self::Read, Self::Grep, Self::Glob];

    /// Whether `claude plugin eval` withholds it unless the operator grants it with
    /// `--allow-tools`.
    pub(crate) const fn is_gated(self) -> bool {
        matches!(self, Self::Write | Self::Edit | Self::Bash)
    }

    /// Its name as Claude Code knows it.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Skill => "Skill",
            Self::Read => "Read",
            Self::Grep => "Grep",
            Self::Glob => "Glob",
            Self::Write => "Write",
            Self::Edit => "Edit",
            Self::Bash => "Bash",
        }
    }
}

/// A case file as written.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseDto {
    behaviour: Behaviour,
    #[serde(default)]
    pressure: Option<Pressure>,
    /// Where the case came from: the real failure it was seeded from.
    seed: String,
    prompt: String,
    #[serde(default)]
    max_turns: Option<u32>,
    #[serde(default)]
    timeout_seconds: Option<u32>,
    #[serde(default)]
    allowed_tools: Option<Vec<Tool>>,
    /// The fixture under `<suites>/fixtures/` that seeds each run's workspace.
    #[serde(default)]
    fixture: Option<String>,
    graders: Vec<Grader>,
}

/// A case as read, before it is placed in its skill and suite.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Parsed {
    /// How its runs are scored.
    pub(crate) behaviour: Behaviour,
    /// Its pressure fixture, if any.
    pub(crate) pressure: Option<Pressure>,
    /// The prompt as run: the written prompt, then the pressure sentence.
    pub(crate) prompt: String,
    /// Turns per run.
    pub(crate) max_turns: u32,
    /// Seconds per run.
    pub(crate) timeout_seconds: u32,
    /// The tools its runs may use.
    pub(crate) allowed_tools: Vec<Tool>,
    /// The fixture that seeds each run's workspace, if any.
    pub(crate) fixture: Option<FixtureName>,
    /// Its graders, deterministic ones first.
    pub(crate) graders: Vec<Grader>,
}

/// Why a case file is refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum Fault {
    /// It is not a case: bad JSON, an unknown field, tool or grader type.
    #[error("not a case: {0}")]
    Parse(String),
    /// A required text is blank.
    #[error("the {0} is blank")]
    Blank(&'static str),
    /// The prompt is longer than 8 KiB.
    #[error("the prompt is longer than {MAX_PROMPT_BYTES} bytes")]
    LongPrompt,
    /// It has no grader.
    #[error("the case has no grader")]
    NoGraders,
    /// It has only `tool_used` graders, which check the trajectory, not the outcome.
    #[error(
        "the case needs an outcome grader (tool_order, regex, file_exists or llm), not only tool_used"
    )]
    NoOutcomeGrader,
    /// Two graders share a name.
    #[error("two graders are named `{0}`")]
    DuplicateGrader(String),
    /// An `llm` grader comes before a deterministic one.
    #[error("llm grader `{0}` comes before a deterministic grader; llm graders go last")]
    LlmNotLast(String),
    /// A `regex` grader's `match` is not `contains`, `not_contains` or `count:<n>`.
    #[error("grader `{0}`: match must be contains, not_contains or count:<n>")]
    BadMatch(String),
    /// A `regex` grader's flags are not JavaScript regular-expression flags.
    #[error("grader `{0}`: flags must be among {REGEX_FLAGS}")]
    BadFlags(String),
    /// A weight is not a finite number above zero.
    #[error("grader `{0}`: weight must be a number above zero")]
    BadWeight(String),
    /// `max_turns` is outside 1 to 50.
    #[error("max_turns {0} is outside 1 to {MAX_TURNS}")]
    Turns(u32),
    /// `timeout_seconds` is outside 30 to 1800.
    #[error("timeout_seconds {0} is outside 30 to 1800")]
    Timeout(u32),
    /// `allowed_tools` leaves out `Skill`, so the skill could never load.
    #[error("allowed_tools must include Skill")]
    NoSkillTool,
    /// `fixture` is not a plain name.
    #[error(
        "fixture {0:?} is not a fixture name: 1 to 64 of A-Z, a-z, 0-9, ., _ and -, not \
         starting with ."
    )]
    Fixture(String),
}

/// The case in `bytes`.
///
/// # Errors
/// The first [`Fault`] found.
pub(crate) fn parse(bytes: &[u8]) -> Result<Parsed, Fault> {
    let dto: CaseDto =
        serde_json::from_slice(bytes).map_err(|error| Fault::Parse(error.to_string()))?;
    for (field, text) in [("prompt", &dto.prompt), ("seed", &dto.seed)] {
        if text.trim().is_empty() {
            return Err(Fault::Blank(field));
        }
    }
    let prompt = match dto.pressure {
        Some(pressure) => format!("{}\n\n{}", dto.prompt.trim_end(), pressure.sentence()),
        None => dto.prompt,
    };
    if prompt.len() > MAX_PROMPT_BYTES {
        return Err(Fault::LongPrompt);
    }
    let max_turns = dto.max_turns.unwrap_or(DEFAULT_MAX_TURNS);
    if !(1..=MAX_TURNS).contains(&max_turns) {
        return Err(Fault::Turns(max_turns));
    }
    let timeout_seconds = dto.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS);
    if !TIMEOUT_SECONDS.contains(&timeout_seconds) {
        return Err(Fault::Timeout(timeout_seconds));
    }
    let allowed_tools = dto.allowed_tools.unwrap_or_else(|| Tool::DEFAULT.to_vec());
    if !allowed_tools.contains(&Tool::Skill) {
        return Err(Fault::NoSkillTool);
    }
    let fixture = dto
        .fixture
        .map(FixtureName::try_from)
        .transpose()
        .map_err(Fault::Fixture)?;
    check_graders(&dto.graders)?;
    Ok(Parsed {
        behaviour: dto.behaviour,
        pressure: dto.pressure,
        prompt,
        max_turns,
        timeout_seconds,
        allowed_tools,
        fixture,
        graders: dto.graders,
    })
}

/// The grader rules: at least one, one outcome grader, unique names, `llm` last, valid
/// `match`, flags and weights.
fn check_graders(graders: &[Grader]) -> Result<(), Fault> {
    if graders.is_empty() {
        return Err(Fault::NoGraders);
    }
    let mut names: Vec<&str> = Vec::with_capacity(graders.len());
    let mut llm_seen: Option<&str> = None;
    for grader in graders {
        let name = grader.name();
        if name.trim().is_empty() {
            return Err(Fault::Blank("grader name"));
        }
        if names.contains(&name) {
            return Err(Fault::DuplicateGrader(name.to_owned()));
        }
        names.push(name);
        match (grader, llm_seen) {
            (Grader::Llm(_), _) => llm_seen = Some(name),
            (_, Some(llm)) => return Err(Fault::LlmNotLast(llm.to_owned())),
            (_, None) => {}
        }
        if grader.weight().is_some_and(|w| !(w.is_finite() && w > 0.0)) {
            return Err(Fault::BadWeight(name.to_owned()));
        }
        if let Grader::Regex(regex) = grader {
            if regex.matching.as_deref().is_some_and(|m| !is_match(m)) {
                return Err(Fault::BadMatch(name.to_owned()));
            }
            if regex
                .flags
                .as_deref()
                .is_some_and(|flags| !flags.chars().all(|c| REGEX_FLAGS.contains(c)))
            {
                return Err(Fault::BadFlags(name.to_owned()));
            }
        }
    }
    if graders.iter().all(|g| matches!(g, Grader::ToolUsed(_))) {
        return Err(Fault::NoOutcomeGrader);
    }
    Ok(())
}

/// Whether `text` is a `regex` grader's `match`: `contains`, `not_contains` or `count:<n>`.
fn is_match(text: &str) -> bool {
    match text.strip_prefix("count:") {
        Some(count) => !count.is_empty() && count.bytes().all(|b| b.is_ascii_digit()),
        None => matches!(text, "contains" | "not_contains"),
    }
}

#[cfg(test)]
mod tests;
