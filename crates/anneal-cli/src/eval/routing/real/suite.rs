//! The eval cases `claude plugin eval` runs: pure, no I/O.
//!
//! One case per prompt, written as `case.yaml` in JSON (YAML 1.2 reads JSON as itself, so no
//! YAML writer is needed). Each case has one `tool_used: Skill` grader per model-invoked
//! skill, so a run tells which skills fired, paired with an outcome grader (`answered`: the
//! run ended with a non-empty reply), since a suite of `Skill` graders alone scores nothing.

use serde::Serialize;

/// The case schema version this writes (Claude Code reads up to 1.x).
const SCHEMA_VERSION: &str = "1.0";
/// Turns per run: routing is decided on the first, and the second lets the skill start.
const MAX_TURNS: u32 = 2;
/// Seconds per run before `claude plugin eval` counts it as errored.
const TIMEOUT_SECONDS: u32 = 180;
/// The tool whose calls show which skill fired.
const SKILL_TOOL: &str = "Skill";
/// Prefix of a skill grader's name: `fired-<skill>`.
const FIRED_PREFIX: &str = "fired-";
/// The outcome grader's name.
pub(crate) const ANSWERED: &str = "answered";

/// The graders every case carries: one per skill, then the outcome grader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Graders<'a> {
    /// Each skill with its grader's name and `input_match` pattern.
    skills: Vec<SkillGrader<'a>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SkillGrader<'a> {
    skill: &'a str,
    name: String,
    input_match: String,
}

impl<'a> Graders<'a> {
    /// The graders for `skills` of the plugin named `plugin`. A `Skill` call's input is
    /// matched as Claude Code serializes it (`{"skill":"<plugin>:<skill>"}`), with or without
    /// the plugin's prefix, and never a longer name that starts with the skill's.
    pub(crate) fn new(plugin: &str, skills: &'a [String]) -> Self {
        let prefix = regex_literal(&json_body(plugin));
        Self {
            skills: skills
                .iter()
                .map(|skill| SkillGrader {
                    skill,
                    name: format!("{FIRED_PREFIX}{skill}"),
                    input_match: format!(
                        "[{{,]\"skill\":\"(?:{prefix}:)?{}\"[,}}]",
                        regex_literal(&json_body(skill))
                    ),
                })
                .collect(),
        }
    }

    /// The skill a grader of this set reports on, or `None` for the outcome grader or a
    /// grader it did not write.
    pub(crate) fn skill_of(&self, grader: &str) -> Option<&'a str> {
        self.skills
            .iter()
            .find(|g| g.name == grader)
            .map(|g| g.skill)
    }
}

/// One case as `case.yaml` holds it.
#[derive(Debug, Serialize)]
pub(crate) struct CaseFile<'a> {
    schema_version: &'static str,
    name: &'a str,
    description: &'static str,
    tags: [&'static str; 1],
    execution: Execution<'a>,
    runs: u32,
    graders: Vec<Grader<'a>>,
}

#[derive(Debug, Serialize)]
struct Execution<'a> {
    prompt: &'a str,
    max_turns: u32,
    timeout_seconds: u32,
    allowed_tools: [&'static str; 1],
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Grader<'a> {
    ToolUsed {
        name: &'a str,
        tool: &'static str,
        input_match: &'a str,
    },
    Regex {
        name: &'static str,
        target: &'static str,
        pattern: &'static str,
    },
}

/// The case named `name` that runs `prompt` `runs` times under `graders`.
pub(crate) fn case<'a>(
    name: &'a str,
    prompt: &'a str,
    runs: u32,
    graders: &'a Graders<'_>,
) -> CaseFile<'a> {
    let mut list: Vec<Grader<'a>> = graders
        .skills
        .iter()
        .map(|g| Grader::ToolUsed {
            name: &g.name,
            tool: SKILL_TOOL,
            input_match: &g.input_match,
        })
        .collect();
    list.push(Grader::Regex {
        name: ANSWERED,
        target: "last_message",
        pattern: "\\S",
    });
    CaseFile {
        schema_version: SCHEMA_VERSION,
        name,
        description: "anneal eval routing (Tier 2): which skill fires",
        tags: ["anneal-routing"],
        execution: Execution {
            prompt,
            max_turns: MAX_TURNS,
            timeout_seconds: TIMEOUT_SECONDS,
            allowed_tools: [SKILL_TOOL],
        },
        runs,
        graders: list,
    }
}

/// `text` as it appears inside a JSON string: escaped, without the quotes.
fn json_body(text: &str) -> String {
    let quoted = serde_json::Value::from(text).to_string();
    match quoted
        .strip_prefix('"')
        .and_then(|body| body.strip_suffix('"'))
    {
        Some(body) => body.to_owned(),
        None => quoted,
    }
}

/// `text` as a JavaScript regular expression that matches it literally.
fn regex_literal(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        if "\\^$.|?*+()[]{}".contains(c) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Graders, case};

    #[test]
    fn a_case_has_one_skill_grader_per_skill_and_the_outcome_grader() {
        let skills = ["plan", "spec"].map(str::to_owned);
        let graders = Graders::new("anneal", &skills);

        let value =
            serde_json::to_value(case("prompt-001", "break the spec into tasks", 3, &graders))
                .expect("serializes");

        assert_eq!(
            value,
            json!({
                "schema_version": "1.0",
                "name": "prompt-001",
                "description": "anneal eval routing (Tier 2): which skill fires",
                "tags": ["anneal-routing"],
                "execution": {
                    "prompt": "break the spec into tasks",
                    "max_turns": 2,
                    "timeout_seconds": 180,
                    "allowed_tools": ["Skill"]
                },
                "runs": 3,
                "graders": [
                    {
                        "type": "tool_used",
                        "name": "fired-plan",
                        "tool": "Skill",
                        "input_match": "[{,]\"skill\":\"(?:anneal:)?plan\"[,}]"
                    },
                    {
                        "type": "tool_used",
                        "name": "fired-spec",
                        "tool": "Skill",
                        "input_match": "[{,]\"skill\":\"(?:anneal:)?spec\"[,}]"
                    },
                    {
                        "type": "regex",
                        "name": "answered",
                        "target": "last_message",
                        "pattern": "\\S"
                    }
                ]
            })
        );
    }

    #[test]
    fn regex_metacharacters_in_names_match_literally() {
        let skills = ["a.b".to_owned()];
        let graders = Graders::new("my+plugin", &skills);

        let value = serde_json::to_value(case("prompt-001", "x", 3, &graders)).expect("serializes");

        assert_eq!(
            value["graders"][0]["input_match"],
            json!("[{,]\"skill\":\"(?:my\\+plugin:)?a\\.b\"[,}]")
        );
    }

    #[test]
    fn the_skill_grader_names_map_back_to_their_skills() {
        let skills = ["plan".to_owned()];
        let graders = Graders::new("anneal", &skills);

        assert_eq!(
            (
                graders.skill_of("fired-plan"),
                graders.skill_of("answered"),
                graders.skill_of("fired-x")
            ),
            (Some("plan"), None, None)
        );
    }
}
