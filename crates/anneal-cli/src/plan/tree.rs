//! `plan approve`'s tree check (#77): a spec whose header links a design tree (`Tree:`) is
//! approved only when one of the latest `grill_confirmed` events carries the tree's current
//! hash. A spec with no `Tree:` line is not checked, so specs written before the line existed
//! still approve.
//!
//! The check runs in two steps, so that what the files alone decide is decided before the
//! ledger is located: [`read`] refuses a `Tree:` line that names no readable tree, then
//! [`confirmed`] opens the ledger (once, for the approval to use) and looks for the hash.

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

/// A design tree read for the check: where it is, and the `sha256:` hash of its bytes.
#[derive(Debug)]
pub(super) struct Tree {
    /// The tree's path, joined to the spec's directory.
    path: PathBuf,
    hash: String,
}

/// What reading the linked design tree found.
#[derive(Debug)]
pub(super) enum Read {
    /// The tree, still to be found confirmed.
    Tree(Tree),
    /// The approval is refused: the line names no tree, or none that can be read.
    Refused(Refusal),
}

/// Reads the design tree `link`, read from the spec at `spec`. Opens no ledger.
///
/// # Errors
/// [`CliError::Grill`] when the tree exists but cannot be read.
pub(super) fn read(spec: &Path, link: &TreeLink) -> Result<Read, CliError> {
    let spec_shown = spec.display().to_string();
    let TreeLink::Named { target, .. } = link else {
        return Ok(Read::Refused(unlinked(
            &spec_shown,
            None,
            "its `Tree:` line names no design tree",
        )));
    };
    let path = spec::tree_path(&spec_shown, target);
    match grill::read_tree(Path::new(""), &path) {
        Ok(bytes) => Ok(Read::Tree(Tree {
            path,
            hash: sha256(&bytes),
        })),
        Err(error @ (GrillError::Missing { .. } | GrillError::NotAFile { .. })) => {
            Ok(Read::Refused(unlinked(
                &spec_shown,
                Some(path),
                &format!(
                    "its `Tree:` line names no readable design tree ({})",
                    reason(&error)
                ),
            )))
        }
        Err(error) => Err(error.into()),
    }
}

/// What looking for the tree's confirmation found.
#[derive(Debug)]
pub(super) enum Confirmation {
    /// The tree at `path` is confirmed at its current hash in `ledger`, opened for the
    /// approval.
    Confirmed {
        /// The ledger, open.
        ledger: Store,
        /// The tree's path.
        path: PathBuf,
    },
    /// The approval is refused: no confirmation carries the tree's current hash.
    Refused(Refusal),
}

/// Looks for a confirmation of `tree` in the ledger at `store`. A ledger that does not exist
/// holds no confirmation and is not created.
///
/// # Errors
/// [`CliError::Ledger`] when the ledger cannot be opened or read.
pub(super) fn confirmed(tree: Tree, store: &Path) -> Result<Confirmation, CliError> {
    // Opening a ledger that does not exist would create it; it holds no confirmation.
    if let Err(error) = fs::metadata(store)
        && error.kind() == io::ErrorKind::NotFound
    {
        return Ok(Confirmation::Refused(unconfirmed(tree.path)));
    }
    let ledger = Store::open(store)?;
    if grill::is_confirmed(&ledger, &tree.hash)? {
        Ok(Confirmation::Confirmed {
            ledger,
            path: tree.path,
        })
    } else {
        Ok(Confirmation::Refused(unconfirmed(tree.path)))
    }
}

/// The refusal for the design tree at `path`, which no `grill_confirmed` carries at its
/// current hash.
fn unconfirmed(path: PathBuf) -> Refusal {
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
    Refusal {
        tree: Some(path),
        ..Refusal::of(finding)
    }
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
