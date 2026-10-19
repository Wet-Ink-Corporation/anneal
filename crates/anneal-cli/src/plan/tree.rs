//! `plan approve`'s tree check (#77): a spec whose header links a design tree (`Tree:`) is
//! approved only when one of the latest `grill_confirmed` events carries the tree's current
//! hash. A spec with no `Tree:` line is not checked, so specs written before the line existed
//! still approve.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anneal_ledger::Store;

use super::Refusal;
use crate::content_hash::sha256;
use crate::envelope::Finding;
use crate::error::CliError;
use crate::error_chain::reason;
use crate::grill::{self, GrillError};
use crate::lint::spec::{self, TreeLink};

/// What the tree check found.
#[derive(Debug)]
pub(super) enum TreeCheck {
    /// The tree at this path is confirmed at its current hash.
    Confirmed(PathBuf),
    /// The approval is refused.
    Refused(Refusal),
}

/// Checks the design tree `link`, read from the spec at `spec`, against the ledger at `store`.
/// A ledger that does not exist holds no confirmation and is not created.
///
/// # Errors
/// [`CliError::Grill`] when the tree exists but cannot be read, [`CliError::Ledger`] when the
/// ledger cannot be opened or read.
pub(super) fn check(spec: &Path, link: &TreeLink, store: &Path) -> Result<TreeCheck, CliError> {
    let spec_shown = spec.display().to_string();
    let TreeLink::Named { target, .. } = link else {
        return Ok(TreeCheck::Refused(unlinked(
            &spec_shown,
            None,
            "its `Tree:` line names no design tree",
        )));
    };
    let path = spec::tree_path(&spec_shown, target);
    let bytes = match grill::read_tree(Path::new(""), &path) {
        Ok(bytes) => bytes,
        Err(error @ (GrillError::Missing { .. } | GrillError::NotAFile { .. })) => {
            return Ok(TreeCheck::Refused(unlinked(
                &spec_shown,
                Some(path),
                &format!(
                    "its `Tree:` line names no readable design tree ({})",
                    reason(&error)
                ),
            )));
        }
        Err(error) => return Err(error.into()),
    };
    // Opening a ledger that does not exist would create it; it holds no confirmation.
    let confirmed = match fs::metadata(store) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Ok(_) | Err(_) => grill::is_confirmed(&Store::open(store)?, &sha256(&bytes))?,
    };
    if confirmed {
        return Ok(TreeCheck::Confirmed(path));
    }
    let shown = path.display().to_string();
    let finding = Finding::new(
        format!("plan/approve/unconfirmed-tree:{shown}"),
        format!(
            "{shown}: the spec's design tree has no `grill_confirmed` with its current hash; \
             plan approval needs the human's confirmation of the tree the spec was written \
             from. fix: have the human confirm it, then run `anneal grill confirm --tree \
             {shown} --quote \"<the human's words>\"` and approve again."
        ),
    );
    Ok(TreeCheck::Refused(Refusal {
        tree: Some(path),
        ..Refusal::of(finding)
    }))
}

/// The refusal for a spec whose `Tree:` line `problem` describes.
fn unlinked(spec: &str, tree: Option<PathBuf>, problem: &str) -> Refusal {
    let finding = Finding::new(
        format!("plan/approve/tree:{spec}"),
        format!(
            "{spec}: {problem}. fix: name the design tree the spec was written from, relative \
             to the spec, then run `anneal lint spec {spec}` and approve again."
        ),
    );
    Refusal {
        tree,
        ..Refusal::of(finding)
    }
}
