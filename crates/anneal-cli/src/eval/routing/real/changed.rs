//! `--changed-since <REF>`: whether anything the router reads changed since a base revision.
//!
//! The router sees each model-invoked skill's `name`, `description` and `when_to_use`, so a
//! run is needed when one of those changed in the working tree since `REF`, a skill was added
//! or removed, or a skill's `disable-model-invocation` flipped. A body edit is not a routing
//! change. Anything that cannot be read on either side counts as a change, so a doubtful
//! comparison runs the suite rather than skipping it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::exit_codes::{self, Code};
use crate::lint::skills::LintError;
use crate::lint::skills::discover::{EntryKind, MAX_READ_BYTES, list_dir, read_bounded};
use crate::lint::skills::frontmatter;
use crate::paths::{PathError, run_git_capped};

/// How long one git call may take.
const GIT_TIMEOUT: Duration = Duration::from_secs(10);

/// Why `--changed-since` could not compare.
#[derive(Debug, thiserror::Error)]
pub enum ChangedError {
    /// The base names no commit.
    #[error("--changed-since {base:?} names no commit")]
    Unresolved {
        /// The base as given.
        base: String,
    },
    /// git could not be run.
    #[error("comparing skills with git")]
    Git(#[source] PathError),
    /// The skills directory could not be listed.
    #[error("listing the skills")]
    List(#[source] LintError),
    /// A skill file could not be read.
    #[error("reading {}", path.display())]
    Read {
        /// The file.
        path: PathBuf,
        /// Why.
        #[source]
        source: std::io::Error,
    },
}

impl ChangedError {
    /// The documented exit code: a base that names no commit is a usage error, the rest
    /// internal.
    #[must_use]
    pub const fn code(&self) -> Code {
        match self {
            Self::Unresolved { .. } => exit_codes::USAGE,
            Self::Git(_) | Self::List(_) | Self::Read { .. } => exit_codes::INTERNAL,
        }
    }
}

/// What the router reads of one skill; `None` for a user-invoked skill, which it never sees.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Routing {
    /// The frontmatter did not parse, or the file could not be read.
    Unreadable,
    /// A model-invoked skill's listing.
    Listed {
        name: String,
        description: String,
        when_to_use: String,
    },
}

/// What the router reads of a skill file's `bytes` (`None`: too large to read), or `None`
/// for a user-invoked skill, which the router never lists: the same as no skill at all.
fn routing_of(bytes: Option<Vec<u8>>) -> Option<Routing> {
    let text = bytes.and_then(|bytes| String::from_utf8(bytes).ok());
    let Some(doc) = text
        .as_deref()
        .and_then(|text| frontmatter::parse(text).ok())
    else {
        return Some(Routing::Unreadable);
    };
    if doc.value("disable-model-invocation") == "true" {
        return None;
    }
    Some(Routing::Listed {
        name: doc.value("name").to_owned(),
        description: doc.value("description").to_owned(),
        when_to_use: doc.value("when_to_use").to_owned(),
    })
}

/// Whether any skill under `skills` reads differently to the router than at `base`.
///
/// # Errors
/// [`ChangedError::Unresolved`] when `base` names no commit; [`ChangedError::Git`] when git
/// cannot be run; [`ChangedError::Skills`] when the directory cannot be listed.
pub(crate) fn changed(skills: &Path, base: &str) -> Result<bool, ChangedError> {
    let commit = format!("{base}^{{commit}}");
    let resolved = git(
        skills,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &commit,
        ],
    )?;
    if resolved.is_none() {
        return Err(ChangedError::Unresolved {
            base: base.to_owned(),
        });
    }
    // No tree at the base means the directory is new: every skill in it is added.
    // From inside the directory, `ls-tree <base> -- ./` lists its entries by name.
    let listing = git(skills, &["ls-tree", "-z", "--name-only", base, "--", "./"])?;
    let mut names: BTreeSet<String> = listing
        .iter()
        .flat_map(|listing| listing.split(|&byte| byte == 0))
        .filter(|name| !name.is_empty())
        .map(|name| String::from_utf8_lossy(name).into_owned())
        .collect();
    let entries = list_dir(skills).map_err(ChangedError::List)?;
    names.extend(
        entries
            .into_iter()
            .filter(|(_, kind)| matches!(kind, EntryKind::Dir | EntryKind::Symlink))
            .map(|(name, _)| name.to_string_lossy().into_owned()),
    );
    for name in names {
        let before = git(skills, &["show", &format!("{base}:./{name}/SKILL.md")])?
            .and_then(|bytes| routing_of(Some(bytes)));
        let path = skills.join(&name).join("SKILL.md");
        let now = if path.is_file() {
            let read = read_bounded(&path).map_err(|source| ChangedError::Read { path, source })?;
            routing_of(read.bytes)
        } else {
            None
        };
        if before != now {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Runs `git <args>` in `dir`: its stdout on success, `None` on a non-zero exit (a path or
/// revision that does not exist), or output over the read cap, which is treated as unknown.
fn git(dir: &Path, args: &[&str]) -> Result<Option<Vec<u8>>, ChangedError> {
    let cap = MAX_READ_BYTES.saturating_add(1);
    let output = run_git_capped(dir, args, GIT_TIMEOUT, cap).map_err(ChangedError::Git)?;
    let complete = u64::try_from(output.stdout.len()).is_ok_and(|len| len < cap);
    Ok((output.status.success() && complete).then_some(output.stdout))
}

#[cfg(test)]
mod tests {
    use super::{ChangedError, changed};
    use crate::test_repo::{ScratchRepo, Setup};

    const PLAN: &str = "---\nname: plan\ndescription: Use when planning tasks.\n---\n\nBody.\n";

    fn repo_with_plan() -> Setup<(ScratchRepo, String)> {
        let repo = ScratchRepo::new()?;
        repo.write("plugin/skills/plan/SKILL.md", PLAN)?;
        let base = repo.commit("plan")?;
        Ok((repo, base))
    }

    #[test]
    fn a_body_edit_is_no_routing_change() -> Setup<()> {
        let (repo, base) = repo_with_plan()?;
        repo.write(
            "plugin/skills/plan/SKILL.md",
            &PLAN.replace("Body.", "New body."),
        )?;

        assert!(!changed(&repo.path().join("plugin/skills"), &base)?);
        Ok(())
    }

    #[test]
    fn a_description_edit_is_a_change_committed_or_not() -> Setup<()> {
        let (repo, base) = repo_with_plan()?;
        repo.write(
            "plugin/skills/plan/SKILL.md",
            &PLAN.replace("planning tasks", "ordering tasks"),
        )?;
        let uncommitted = changed(&repo.path().join("plugin/skills"), &base)?;
        repo.commit("reword")?;

        assert_eq!(
            (
                uncommitted,
                changed(&repo.path().join("plugin/skills"), &base)?
            ),
            (true, true)
        );
        Ok(())
    }

    #[test]
    fn an_added_or_removed_skill_is_a_change() -> Setup<()> {
        let (repo, base) = repo_with_plan()?;
        repo.write(
            "plugin/skills/spec/SKILL.md",
            "---\nname: spec\ndescription: Use when specifying.\n---\n",
        )?;
        let added = changed(&repo.path().join("plugin/skills"), &base)?;
        let spec_base = repo.commit("spec")?;
        std::fs::remove_dir_all(repo.path().join("plugin/skills/spec"))?;

        assert_eq!(
            (
                added,
                changed(&repo.path().join("plugin/skills"), &spec_base)?
            ),
            (true, true)
        );
        Ok(())
    }

    #[test]
    fn a_user_invoked_skill_never_routes_so_its_description_is_no_change() -> Setup<()> {
        let repo = ScratchRepo::new()?;
        let release = "---\nname: release\ndescription: Cut a release.\ndisable-model-invocation: true\n---\n";
        repo.write("plugin/skills/release/SKILL.md", release)?;
        let base = repo.commit("release")?;
        repo.write(
            "plugin/skills/release/SKILL.md",
            &release.replace("Cut a release", "Ship a release"),
        )?;

        assert!(!changed(&repo.path().join("plugin/skills"), &base)?);
        Ok(())
    }

    #[test]
    fn a_base_that_names_no_commit_is_refused() -> Setup<()> {
        let (repo, _) = repo_with_plan()?;

        let refused = changed(&repo.path().join("plugin/skills"), "no-such-ref");

        assert!(
            matches!(&refused, Err(ChangedError::Unresolved { base }) if base == "no-such-ref"),
            "{refused:?}"
        );
        Ok(())
    }
}
