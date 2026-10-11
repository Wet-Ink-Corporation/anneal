//! The route playbook, `plugin/playbooks/route.json`: the phases D3 chooses from, each with
//! its entry skill (if one has landed), the phase that stands for "no workflow skill", and the
//! size levels D3 scores, each with its depth.
//!
//! The file is built into the binary and read strictly: version 1, no unknown field, at least
//! two distinct phases and one size, every phase name a decide option word (it is sent as a
//! `choice` option and written back as an outcome label), and a `no_skill_phase` that is a
//! phase without an entry skill.

use std::collections::BTreeSet;

use serde::Deserialize;

use crate::decide::client::MAX_OPTIONS;
use crate::decide::id::is_word;

/// `plugin/playbooks/route.json`, as built in.
const BUILT_IN: &str = include_str!("../../../../../plugin/playbooks/route.json");

/// The format version read.
const VERSION: u32 = 1;

/// The prefix a plugin skill's name carries in Claude Code's `Skill` tool input.
const PLUGIN_PREFIX: &str = "anneal:";

/// The phases and sizes D3 asks about.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Playbook {
    version: u32,
    phases: Vec<Phase>,
    no_skill_phase: String,
    sizes: Vec<Size>,
}

/// A phase and the skill that starts it, when that skill has landed.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Phase {
    phase: String,
    entry_skill: Option<String>,
}

/// A size level and the depth of process it calls for.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Size {
    size: String,
    depth: String,
}

/// Why the playbook could not be read.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PlaybookError {
    /// Not JSON of the playbook's shape.
    #[error("route.json does not parse")]
    Json(#[source] serde_json::Error),
    /// The JSON parses but breaks a rule.
    #[error("route.json: {0}")]
    Invalid(&'static str),
}

impl Playbook {
    /// The playbook built into the binary.
    ///
    /// # Errors
    /// [`PlaybookError`] if the built-in file is invalid (a test keeps it valid).
    pub(crate) fn built_in() -> Result<Self, PlaybookError> {
        Self::parse(BUILT_IN)
    }

    /// Reads a playbook from `text`, strictly.
    ///
    /// # Errors
    /// [`PlaybookError::Json`] for text of another shape, [`PlaybookError::Invalid`] for a
    /// playbook that breaks one of the module's rules.
    pub(crate) fn parse(text: &str) -> Result<Self, PlaybookError> {
        let playbook: Self = serde_json::from_str(text).map_err(PlaybookError::Json)?;
        playbook.validate()?;
        Ok(playbook)
    }

    fn validate(&self) -> Result<(), PlaybookError> {
        let invalid = |reason| Err(PlaybookError::Invalid(reason));
        if self.version != VERSION {
            return invalid("version must be 1");
        }
        let names: BTreeSet<&str> = self.phases.iter().map(Phase::name).collect();
        if !(2..=MAX_OPTIONS).contains(&self.phases.len()) || names.len() != self.phases.len() {
            return invalid("phases must be 2 to 32 distinct names");
        }
        if !names.iter().all(|name| is_word(name)) {
            return invalid("a phase name must be 1 to 64 of A-Z a-z 0-9 _ . : -");
        }
        let no_skill = self.phases.iter().find(|p| p.phase == self.no_skill_phase);
        if no_skill.is_none_or(|phase| phase.entry_skill.is_some()) {
            return invalid("no_skill_phase must name a phase without an entry skill");
        }
        if self.sizes.is_empty() {
            return invalid("sizes must not be empty");
        }
        Ok(())
    }

    /// The phases, in file order.
    pub(crate) fn phases(&self) -> impl Iterator<Item = &Phase> + '_ {
        self.phases.iter()
    }

    /// The phase that stands for "no workflow skill was needed".
    pub(crate) fn no_skill_phase(&self) -> &str {
        &self.no_skill_phase
    }

    /// The phase named `name`, if any.
    pub(crate) fn phase(&self, name: &str) -> Option<&Phase> {
        self.phases.iter().find(|phase| phase.phase == name)
    }

    /// The phase whose entry skill is `skill`, written bare (`build`) or as the plugin's
    /// (`anneal:build`).
    pub(crate) fn phase_of_skill(&self, skill: &str) -> Option<&Phase> {
        let bare = skill.strip_prefix(PLUGIN_PREFIX).unwrap_or(skill);
        self.phases
            .iter()
            .find(|phase| phase.entry_skill.as_deref() == Some(bare))
    }

    /// The size level nearest to `score` (0 to 1, the levels evenly spaced over it); `None`
    /// only for a playbook with no sizes, which [`Playbook::parse`] refuses.
    pub(crate) fn size_for(&self, score: f64) -> Option<&Size> {
        let steps = u32::try_from(self.sizes.len().saturating_sub(1)).ok()?;
        // Level i sits at i / steps, so the score scaled by steps is nearest to level i when
        // it is below i + 0.5.
        let scaled = score * f64::from(steps);
        let mut midpoint = 0.5_f64;
        for size in &self.sizes {
            if scaled < midpoint {
                return Some(size);
            }
            midpoint += 1.0;
        }
        self.sizes.last()
    }
}

impl Phase {
    /// The phase's name, as sent to Clef and written as a label.
    pub(crate) fn name(&self) -> &str {
        &self.phase
    }

    /// The skill that starts the phase, when it has landed.
    pub(crate) fn entry_skill(&self) -> Option<&str> {
        self.entry_skill.as_deref()
    }
}

impl Size {
    /// The size level's name.
    pub(crate) fn name(&self) -> &str {
        &self.size
    }

    /// The depth of process the size calls for.
    pub(crate) fn depth(&self) -> &str {
        &self.depth
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Phase, Playbook, Size};
    use crate::lint::skills::checker::ENTRY_SKILLS;

    #[test]
    fn the_built_in_playbook_has_eight_phases_and_four_sizes() {
        let playbook = Playbook::built_in().expect("route.json parses");

        let phases: Vec<&str> = playbook.phases().map(Phase::name).collect();
        let sizes: Vec<&str> = playbook.sizes.iter().map(Size::name).collect();

        assert_eq!(
            phases,
            [
                "define", "specify", "plan", "build", "diagnose", "verify", "ship", "answer"
            ]
        );
        assert_eq!(sizes, ["trivial", "small", "medium", "large"]);
        assert_eq!(playbook.no_skill_phase(), "answer");
    }

    #[test]
    fn every_entry_skill_is_a_shipped_entry_skill() {
        let playbook = Playbook::built_in().expect("route.json parses");
        let skills = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("plugin")
            .join("skills");

        for phase in playbook.phases() {
            let Some(skill) = phase.entry_skill() else {
                continue;
            };
            assert!(ENTRY_SKILLS.contains(&skill), "{skill} is an entry skill");
            assert!(
                skills.join(skill).join("SKILL.md").is_file(),
                "{skill} has a SKILL.md"
            );
        }
    }

    #[test]
    fn every_shipped_entry_skill_is_the_entry_of_exactly_one_phase() {
        let playbook = Playbook::built_in().expect("route.json parses");
        let skills = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("plugin")
            .join("skills");

        for skill in ENTRY_SKILLS {
            let shipped = skills.join(skill).join("SKILL.md").is_file();
            let entries = playbook
                .phases()
                .filter(|phase| phase.entry_skill() == Some(skill))
                .count();
            assert_eq!(entries, usize::from(shipped), "{skill}");
        }
    }

    #[test]
    fn the_phase_of_a_skill_is_found_by_its_bare_or_plugin_name() {
        let playbook = Playbook::built_in().expect("route.json parses");

        let phase_of = |skill| playbook.phase_of_skill(skill).map(Phase::name);

        assert_eq!(phase_of("build"), Some("build"));
        assert_eq!(phase_of("anneal:grill"), Some("define"));
        assert_eq!(phase_of("temper:rust-craft"), None);
        assert_eq!(phase_of("other:build"), None);
    }

    #[test]
    fn a_score_maps_to_the_nearest_of_the_four_size_levels() {
        let playbook = Playbook::built_in().expect("route.json parses");

        let levels: Vec<&str> = [0.0, 0.16, 0.17, 0.49, 0.5, 0.83, 0.84, 1.0]
            .into_iter()
            .filter_map(|score| playbook.size_for(score).map(Size::name))
            .collect();

        assert_eq!(
            levels,
            [
                "trivial", "trivial", "small", "small", "medium", "medium", "large", "large"
            ]
        );
        assert_eq!(playbook.size_for(0.5).map(Size::depth), Some("standard"));
    }

    #[test]
    fn a_playbook_naming_an_unknown_no_skill_phase_is_refused() {
        let text = r#"{"version": 1, "phases": [{"phase": "a", "entry_skill": null},
            {"phase": "b", "entry_skill": "x"}], "no_skill_phase": "c",
            "sizes": [{"size": "s", "depth": "d"}]}"#;

        assert!(Playbook::parse(text).is_err(), "c is not a phase");
    }

    #[test]
    fn a_playbook_with_unknown_fields_or_bad_names_is_refused() {
        for text in [
            r#"{"version": 1, "phases": [{"phase": "a", "entry_skill": null}], "no_skill_phase": "a", "sizes": [{"size": "s", "depth": "d"}], "extra": 1}"#,
            r#"{"version": 2, "phases": [{"phase": "a", "entry_skill": null}], "no_skill_phase": "a", "sizes": [{"size": "s", "depth": "d"}]}"#,
            r#"{"version": 1, "phases": [{"phase": "a b", "entry_skill": null}], "no_skill_phase": "a b", "sizes": [{"size": "s", "depth": "d"}]}"#,
            r#"{"version": 1, "phases": [{"phase": "a", "entry_skill": null}], "no_skill_phase": "a", "sizes": []}"#,
            r#"{"version": 1, "phases": [{"phase": "a", "entry_skill": null}, {"phase": "a", "entry_skill": null}], "no_skill_phase": "a", "sizes": [{"size": "s", "depth": "d"}]}"#,
        ] {
            assert!(Playbook::parse(text).is_err(), "{text}");
        }
    }
}
