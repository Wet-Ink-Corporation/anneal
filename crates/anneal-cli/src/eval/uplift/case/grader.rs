//! The graders a case may use: `claude plugin eval`'s own grader types, read strictly and
//! written back unchanged (checked against Claude Code 2.1.296's case schema, version 1.1).
//! Its `baseline` type and `arm` field are left out: Tier 3 runs each arm as its own
//! single-arm eval, so neither has a meaning here.

use serde::{Deserialize, Serialize};

/// One grader.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum Grader {
    /// One tool call came before another.
    ToolOrder(ToolOrder),
    /// A pattern in the trace, the last message, the created files or a file's text.
    Regex(Regex),
    /// A file was created (or not).
    FileExists(FileExists),
    /// A tool was called (a trajectory check, not an outcome).
    ToolUsed(ToolUsed),
    /// A model judges the reply against criteria; costs money, so it comes last.
    Llm(Llm),
}

impl Grader {
    /// The grader's name, unique within its case.
    pub(crate) fn name(&self) -> &str {
        match self {
            Self::ToolOrder(g) => &g.name,
            Self::Regex(g) => &g.name,
            Self::FileExists(g) => &g.name,
            Self::ToolUsed(g) => &g.name,
            Self::Llm(g) => &g.name,
        }
    }

    /// Its weight, if given.
    pub(crate) const fn weight(&self) -> Option<f64> {
        match self {
            Self::ToolOrder(g) => g.weight,
            Self::Regex(g) => g.weight,
            Self::FileExists(g) => g.weight,
            Self::ToolUsed(g) => g.weight,
            Self::Llm(g) => g.weight,
        }
    }
}

/// `tool_order`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolOrder {
    pub(crate) name: String,
    pub(crate) before: Step,
    pub(crate) after: Step,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) weight: Option<f64>,
}

/// A tool call in a `tool_order`: the tool's name, or the name with a pattern on its input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum Step {
    /// Any call of the tool.
    Tool(String),
    /// A call of the tool whose input matches.
    Matched(MatchedStep),
}

/// A tool call whose input matches a pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MatchedStep {
    pub(crate) tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) input_match: Option<String>,
}

/// `regex`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Regex {
    pub(crate) name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) target: Option<Target>,
    pub(crate) pattern: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) flags: Option<String>,
    #[serde(default, rename = "match", skip_serializing_if = "Option::is_none")]
    pub(crate) matching: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) weight: Option<f64>,
}

/// What a `regex` or `llm` grader reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum Target {
    /// A part of the run.
    Named(NamedTarget),
    /// A file's text.
    File(FileTarget),
}

/// The parts of a run a grader can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NamedTarget {
    /// The whole trace.
    Trace,
    /// The reply.
    LastMessage,
    /// The paths of the files created.
    Files,
    /// Calls to mocked servers.
    MockCalls,
}

/// A file's text as a target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileTarget {
    pub(crate) source: FileSource,
    pub(crate) path: String,
}

/// The one source of a file target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FileSource {
    /// A file in the run's workspace.
    File,
}

/// `file_exists`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileExists {
    pub(crate) name: String,
    pub(crate) path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) exists: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) weight: Option<f64>,
}

/// `tool_used`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolUsed {
    pub(crate) name: String,
    pub(crate) tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) input_match: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) min: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) max: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) weight: Option<f64>,
}

/// `llm`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Llm {
    pub(crate) name: String,
    pub(crate) criteria: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) focus: Option<Target>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) weight: Option<f64>,
}
