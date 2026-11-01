//! The bootstrap index: the stable part of what the `SessionStart` hook injects.
//!
//! It is what anneal is, the entry skills with their triggers, and the commands only the human
//! runs, in 200 words or fewer (skill rule 4). The entry skills are the linter's
//! `ENTRY_SKILLS`, in its order (a test holds them equal), so `lint skills` and the bootstrap
//! agree on them. The same text is committed as `docs/bootstrap-index.md`, which
//! `anneal lint skills --index` checks.

use std::fmt::{self, Display, Formatter};

/// Each entry skill and the trigger that should load it, in `ENTRY_SKILLS` order.
const ENTRY_TRIGGERS: [(&str, &str); 7] = [
    (
        "grill",
        "a request is new or vague; question it until the requirements are confirmed.",
    ),
    (
        "spec",
        "requirements are agreed; write the spec and its acceptance criteria.",
    ),
    (
        "plan",
        "a spec is ready; split it into tasks with a dependency graph and get the plan approved.",
    ),
    (
        "build",
        "an approved task is next; implement it test-first.",
    ),
    (
        "diagnose",
        "something is broken or a test fails unexpectedly; reproduce it red before fixing.",
    ),
    (
        "verify",
        "work looks done; check it against the spec before claiming it.",
    ),
    (
        "ship",
        "verified work needs a pull request, review verdict, merge or rollout.",
    ),
];

/// The user-invoked skills, as typed, and what each does.
const USER_COMMANDS: [(&str, &str); 5] = [
    (
        "/anneal:orchestrate",
        "run a plan's tasks across subagents.",
    ),
    ("/anneal:handoff", "write a brief for the next session."),
    (
        "/anneal:correct",
        "record a correction as a standing order.",
    ),
    (
        "/anneal:write-skill",
        "author a skill that passes `anneal lint skills`.",
    ),
    ("/anneal:calibrate", "calibrate a Clef decision point."),
];

const PREAMBLE: &str = "\
# anneal

anneal turns SDLC gates into enforced checks. Skills say how to work, the `anneal` CLI and its
hooks check that it was done, and the ledger records what happened.

## Entry skills

Load the skill whose trigger matches before acting:

";

const COMMANDS_HEADING: &str = "
## User commands

The human runs these; do not start them unasked:

";

/// The prefix of a user command naming the skill it runs.
const COMMAND_PREFIX: &str = "/anneal:";

/// The bootstrap index with every skill, as committed as `docs/bootstrap-index.md` and
/// injected when the plugin's skills cannot be listed.
#[must_use]
pub(crate) fn index() -> String {
    index_with(|_| true)
}

/// The bootstrap index listing only the entry skills and user commands whose skill `present`
/// holds, by directory name (`grill`, `handoff`), so a plugin missing a skill never sends the
/// model to load it.
#[must_use]
pub(crate) fn index_with(present: impl Fn(&str) -> bool) -> String {
    let skills = Bullets(&ENTRY_TRIGGERS, &present);
    let commands = Bullets(&USER_COMMANDS, &|command| present(skill_of(command)));
    format!("{PREAMBLE}{skills}{COMMANDS_HEADING}{commands}")
}

/// The directory name of every skill the index can list, entry skills then user commands.
pub(crate) fn skill_names() -> impl Iterator<Item = &'static str> {
    ENTRY_TRIGGERS
        .iter()
        .map(|(name, _)| *name)
        .chain(USER_COMMANDS.iter().map(|(command, _)| skill_of(command)))
}

/// The skill a user command runs: `/anneal:handoff` runs `handoff`.
fn skill_of(command: &str) -> &str {
    command.strip_prefix(COMMAND_PREFIX).unwrap_or(command)
}

/// One ``- `name`: text`` line per pair whose name the filter keeps.
struct Bullets<'a>(&'a [(&'a str, &'a str)], &'a dyn Fn(&str) -> bool);

impl Display for Bullets<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        for (name, text) in self.0.iter().filter(|(name, _)| (self.1)(name)) {
            writeln!(f, "- `{name}`: {text}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{index, index_with};

    /// Path of the generated copy of the index, relative to the repository root.
    const DOC_PATH: &str = "docs/bootstrap-index.md";
    use crate::lint::skills::checker::ENTRY_SKILLS;
    use std::path::Path;

    /// The skill names on the index's `- `name`: trigger` lines, in order.
    fn listed_skills(text: &str) -> Vec<String> {
        text.lines()
            .filter_map(|line| line.strip_prefix("- `"))
            .filter_map(|rest| rest.split_once("`: "))
            .map(|(name, _)| name.to_owned())
            .filter(|name| !name.starts_with('/'))
            .collect()
    }

    #[test]
    fn the_index_is_at_most_200_words() {
        let words = index().split_whitespace().count();
        assert!(words <= 200, "the index is {words} words, budget 200");
    }

    #[test]
    fn the_index_names_every_entry_skill_in_the_linters_order() {
        assert_eq!(
            listed_skills(&index()),
            ENTRY_SKILLS,
            "the index lists exactly the linter's entry skills, in order"
        );
    }

    #[test]
    fn the_index_lists_the_user_invoked_commands_after_the_skills() {
        let text = index();
        let commands = text
            .split_once("## User commands")
            .map(|(_, rest)| rest)
            .expect("a User commands section");
        for command in [
            "/anneal:orchestrate",
            "/anneal:handoff",
            "/anneal:correct",
            "/anneal:write-skill",
            "/anneal:calibrate",
        ] {
            assert!(
                commands.contains(&format!("- `{command}`: ")),
                "{command} is listed under User commands"
            );
        }
    }

    #[test]
    fn a_skill_the_plugin_lacks_is_not_listed_as_an_entry_skill_or_a_command() {
        let text = index_with(|skill| !matches!(skill, "grill" | "handoff"));

        let listed = listed_skills(&text);
        assert_eq!(
            (
                listed.contains(&"grill".to_owned()),
                listed.len(),
                text.contains("/anneal:handoff"),
                text.contains("- `/anneal:correct`: "),
            ),
            (false, ENTRY_SKILLS.len() - 1, false, true),
            "{text}"
        );
    }

    #[test]
    fn bootstrap_index_doc_matches_the_index() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(DOC_PATH);
        let expected = index();
        if std::env::var_os("ANNEAL_BLESS").is_some() {
            std::fs::write(&path, &expected).expect("write docs/bootstrap-index.md");
        }
        let committed = std::fs::read_to_string(&path).expect("docs/bootstrap-index.md exists");
        assert!(
            committed == expected,
            "{DOC_PATH} drifted from the index; regenerate it with \
             `ANNEAL_BLESS=1 cargo test -p anneal-cli bootstrap_index_doc`"
        );
    }
}
