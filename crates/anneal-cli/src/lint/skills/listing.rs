//! Skills as the router lists them, for `eval routing`. Discovery and parsing are the ones
//! `lint skills` uses, so both commands see the same skill set.

use std::path::PathBuf;

use crate::lint::finding::LintFinding;
use crate::lint::skills::document::Skill;
use crate::lint::skills::{LintError, Loaded, discover};

/// A skill as the router lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Listing {
    /// The skill directory's name: the skill's identity and its fixture's file stem.
    pub(crate) name: String,
    /// The words a lexical router sees: the directory name, the `name` field, the trigger (the
    /// description before its "Do NOT use for" clause, whose words point away from the skill)
    /// and `when_to_use`.
    pub(crate) text: String,
    /// False when `disable-model-invocation: true`: only the user can invoke it.
    pub(crate) model_invoked: bool,
}

impl Listing {
    fn of(skill: &Skill<'_>) -> Self {
        let parts = [
            skill.dir.name.as_str(),
            skill.doc.value("name"),
            skill.trigger().trim(),
            skill.doc.value("when_to_use"),
        ];
        Self {
            // The listing outlives the parsed skill and its directory, so it owns its name.
            name: skill.dir.name.clone(),
            text: parts.join(" "),
            model_invoked: !skill.model_invocation_disabled(),
        }
    }
}

/// A skill whose `SKILL.md` could not be read as frontmatter plus body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Unparsed {
    /// The skill directory's name.
    pub(crate) name: String,
    /// The `lint skills` finding that says why.
    pub(crate) finding: LintFinding,
}

/// The skills under `paths`, sorted by directory: each one's listing, or the `lint skills`
/// finding that stops its `SKILL.md` from being read.
///
/// # Errors
/// [`LintError`] when a path is missing or the wrong kind, or a file cannot be read.
pub(crate) fn listings(paths: &[PathBuf]) -> Result<Vec<Result<Listing, Unparsed>>, LintError> {
    let dirs = discover(paths)?;
    let mut found = Vec::with_capacity(dirs.len());
    for dir in &dirs {
        let loaded = Loaded::read(dir)?;
        found.push(
            loaded
                .parse()
                .map(|skill| Listing::of(&skill))
                .map_err(|finding| Unparsed {
                    // The result outlives the directory list, so it owns the name.
                    name: dir.name.clone(),
                    finding,
                }),
        );
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;

    use super::{Listing, listings};
    use crate::lint::skills::checker::Checker;

    #[test]
    fn listings_keep_the_trigger_and_flag_user_invoked_and_unparseable_skills()
    -> Result<(), Box<dyn Error>> {
        let root = tempfile::tempdir()?;
        let write = |name: &str, text: &str| -> std::io::Result<()> {
            fs::create_dir(root.path().join(name))?;
            fs::write(root.path().join(name).join("SKILL.md"), text)
        };
        write(
            "plan",
            "---\nname: plan\ndescription: Use when planning tasks. Do NOT use for specs (use `spec`).\nwhen_to_use: after a spec\n---\n",
        )?;
        write(
            "release",
            "---\nname: release\ndescription: Cut a release.\ndisable-model-invocation: true\n---\n",
        )?;
        write("broken", "no frontmatter\n")?;

        let found = listings(&[root.path().to_path_buf()])?;

        let [broken, plan, release] = found.as_slice() else {
            panic!("three skills, sorted: {found:?}");
        };
        assert_eq!(
            broken
                .as_ref()
                .map_err(|unparsed| (unparsed.name.as_str(), unparsed.finding.checker)),
            Err(("broken", Checker::Frontmatter)),
            "the lint finding that stops parsing"
        );
        assert_eq!(
            plan.as_ref().ok(),
            Some(&Listing {
                name: "plan".to_owned(),
                text: "plan plan Use when planning tasks. after a spec".to_owned(),
                model_invoked: true,
            }),
            "name, trigger and when_to_use; the Do NOT clause is dropped"
        );
        assert_eq!(
            release.as_ref().map(|l| l.model_invoked).ok(),
            Some(false),
            "disable-model-invocation: true"
        );
        Ok(())
    }
}
