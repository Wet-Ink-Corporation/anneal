//! Which suites and cases a run reads: every skill's under `--suites`, narrowed by `--skill`
//! and `--case`.

use std::path::Path;

use super::UpliftError;
use super::args::UpliftArgs;
use super::suites::{self, Case, Suites};
use crate::lint::skills::listing::listings;

/// Reads the suites under `args.suites` for the skills under `args.skills`, keeping only the
/// `--skill`s' cases and coverage when any is named, then only the `--case`'s case and its
/// skill's coverage when it is given.
///
/// # Errors
/// [`UpliftError::Skills`] or [`UpliftError::Suites`] when a directory cannot be read,
/// [`UpliftError::UnknownSkill`] when a `--skill` has no suites, and
/// [`UpliftError::UnknownCase`] when `--case` names none of the cases kept.
pub(super) fn select(args: &UpliftArgs) -> Result<Suites, UpliftError> {
    let skills: Vec<String> = listings(std::slice::from_ref(&args.skills))
        .map_err(UpliftError::Skills)?
        .into_iter()
        .map(|listed| match listed {
            Ok(listing) => listing.name,
            Err(unparsed) => unparsed.name,
        })
        .collect();
    let mut suites = suites::read(&args.suites, &skills).map_err(UpliftError::Suites)?;
    if let Some(name) = args
        .skill
        .iter()
        .find(|name| !suites.coverage.iter().any(|row| &row.skill == *name))
    {
        return Err(UpliftError::UnknownSkill {
            name: name.to_owned(),
        });
    }
    if !args.skill.is_empty() {
        suites.cases.retain(|case| args.skill.contains(&case.skill));
        suites
            .coverage
            .retain(|row| args.skill.contains(&row.skill));
    }
    if let Some(given) = args.case.as_deref() {
        suites.cases.retain(|case| names(case, given, &args.suites));
        let Some(case) = suites.cases.first() else {
            return Err(UpliftError::UnknownCase {
                case: given.to_owned(),
                skills: args.skill.clone(),
            });
        };
        let skill = case.skill.as_str();
        suites.coverage.retain(|row| row.skill == skill);
    }
    Ok(suites)
}

/// Whether `given` (`--case`) names `case`, read from `suites`: it is the case's file under
/// the suites directory, its name, or a path to its file. A path is compared made absolute
/// against the current directory, without touching the filesystem, so `..` and links are not
/// resolved.
fn names(case: &Case, given: &str, suites: &Path) -> bool {
    if given == case.file || given == case.name() {
        return true;
    }
    match (
        std::path::absolute(given),
        std::path::absolute(suites.join(&case.file)),
    ) {
        (Ok(given), Ok(file)) => given == file,
        (Err(_), _) | (_, Err(_)) => false,
    }
}

#[cfg(test)]
mod tests;
