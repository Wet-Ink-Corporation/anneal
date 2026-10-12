//! `--changed-since <REF>`: the skills whose skill directory or suites changed since a base.
//!
//! Tier 3 measures behaviour, so any change to a skill's files (its body included) or to its
//! suites is a change, unlike Tier 2, which reads only the routing fields. Changes are found
//! with `git diff --name-only <base>` in each directory (the working tree against the base,
//! committed or not) plus the untracked files git does not ignore; a path's first component
//! is its skill. A listing too long to read whole counts every skill as changed.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use crate::eval::routing::real::ChangedError;
use crate::paths::{git_failure, run_git_capped};

/// How long one git call may take.
const GIT_TIMEOUT: Duration = Duration::from_secs(10);
/// The most bytes of a listing read (H-13); a longer one counts everything as changed.
const MAX_LISTING_BYTES: u64 = 4 * 1024 * 1024;

/// The skills that changed since a base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Changed {
    /// A listing was too long to read whole, so every skill counts as changed.
    All,
    /// The first path components that changed under either directory.
    Only(BTreeSet<String>),
}

impl Changed {
    /// Whether `skill` changed.
    pub(crate) fn contains(&self, skill: &str) -> bool {
        match self {
            Self::All => true,
            Self::Only(names) => names.contains(skill),
        }
    }
}

/// The skills that changed since `base`, under the skills directory `skill_dir` or the
/// suites directory `suite_dir`.
///
/// # Errors
/// [`ChangedError::Unresolved`] when `base` names no commit; [`ChangedError::Git`] when git
/// cannot be run or fails.
pub(crate) fn changed_skills(
    skill_dir: &Path,
    suite_dir: &Path,
    base: &str,
) -> Result<Changed, ChangedError> {
    let commit = format!("{base}^{{commit}}");
    let resolved = run_git_capped(
        suite_dir,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &commit,
        ],
        GIT_TIMEOUT,
        MAX_LISTING_BYTES,
    )
    .map_err(ChangedError::Git)?;
    if !resolved.status.success() {
        return Err(ChangedError::Unresolved {
            base: base.to_owned(),
        });
    }
    let mut touched = BTreeSet::new();
    for dir in [skill_dir, suite_dir] {
        let diff = [
            "diff",
            "--name-only",
            "-z",
            "--relative",
            "--no-renames",
            "--no-ext-diff",
            base,
            "--",
            ".",
        ];
        let untracked = [
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            ".",
        ];
        for args in [&diff[..], &untracked[..]] {
            let Some(listing) = listing(dir, args)? else {
                return Ok(Changed::All);
            };
            touched.extend(
                listing
                    .split(|&byte| byte == 0)
                    .filter_map(|path| path.split(|&byte| byte == b'/').next())
                    .filter(|first| !first.is_empty())
                    .map(|first| String::from_utf8_lossy(first).into_owned()),
            );
        }
    }
    Ok(Changed::Only(touched))
}

/// `git <args>` in `dir`: its stdout, or `None` when it is too long to read whole.
fn listing(dir: &Path, args: &[&str]) -> Result<Option<Vec<u8>>, ChangedError> {
    let cap = MAX_LISTING_BYTES.saturating_add(1);
    let output = run_git_capped(dir, args, GIT_TIMEOUT, cap).map_err(ChangedError::Git)?;
    if !output.status.success() {
        return Err(ChangedError::Git(git_failure(dir, &output.stderr)));
    }
    let complete = u64::try_from(output.stdout.len()).is_ok_and(|len| len < cap);
    Ok(complete.then_some(output.stdout))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{Changed, changed_skills};
    use crate::eval::routing::real::ChangedError;
    use crate::test_repo::{ScratchRepo, Setup};

    fn names(list: &[&str]) -> BTreeSet<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    fn repo() -> Setup<(ScratchRepo, String)> {
        let repo = ScratchRepo::new()?;
        for skill in ["plan", "spec", "ship"] {
            repo.write(
                &format!("plugin/skills/{skill}/SKILL.md"),
                &format!("---\nname: {skill}\ndescription: Use for {skill}.\n---\n\nBody.\n"),
            )?;
            repo.write(&format!("evals/uplift/{skill}/regression/a.json"), "{}")?;
        }
        let base = repo.commit("base")?;
        Ok((repo, base))
    }

    #[test]
    fn a_body_edit_an_untracked_case_and_a_committed_case_edit_are_changes() -> Setup<()> {
        let (repo, base) = repo()?;
        repo.write("evals/uplift/ship/regression/a.json", "{\"x\": 1}")?;
        repo.commit("ship case")?;
        repo.write(
            "plugin/skills/spec/SKILL.md",
            "---\nname: spec\n---\n\nNew.\n",
        )?;
        repo.write("evals/uplift/plan/capability/b.json", "{}")?;

        let changed = changed_skills(
            &repo.path().join("plugin/skills"),
            &repo.path().join("evals/uplift"),
            &base,
        )?;

        assert_eq!(changed, Changed::Only(names(&["plan", "ship", "spec"])));
        Ok(())
    }

    #[test]
    fn nothing_changed_is_no_skill() -> Setup<()> {
        let (repo, base) = repo()?;

        let changed = changed_skills(
            &repo.path().join("plugin/skills"),
            &repo.path().join("evals/uplift"),
            &base,
        )?;

        assert_eq!(changed, Changed::Only(BTreeSet::new()));
        Ok(())
    }

    #[test]
    fn a_base_that_names_no_commit_is_refused() -> Setup<()> {
        let (repo, _) = repo()?;

        let refused = changed_skills(
            &repo.path().join("plugin/skills"),
            &repo.path().join("evals/uplift"),
            "no-such-ref",
        );

        assert!(
            matches!(&refused, Err(ChangedError::Unresolved { base }) if base == "no-such-ref"),
            "{refused:?}"
        );
        Ok(())
    }
}
