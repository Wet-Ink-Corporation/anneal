//! Opening the ledger only when it already exists, within a caller's time limits: hooks must
//! never create a ledger, and must not wait on git or a locked store past their timeout.
//! Crate-private; the preconditions and fast-check hooks share it.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;
use std::time::Duration;

use anneal_ledger::{Store, StoreError};

use crate::paths::{self, PathError};

/// Why the ledger could not be opened.
#[derive(Debug, thiserror::Error)]
pub(crate) enum LedgerError {
    /// The working directory, needed to find it, cannot be read.
    #[error("reading the working directory")]
    CurrentDir(#[source] io::Error),
    /// git could not say where it is.
    #[error("finding the ledger")]
    Locate(#[source] PathError),
    /// The file exists but the store cannot open it.
    #[error("opening the ledger")]
    Open(#[source] StoreError),
}

/// The ledger seen from `cwd` (found as `paths::ledger_store_within` finds it, git given
/// `git_timeout`) when its file exists, each statement waiting at most `busy_timeout` for a
/// lock; `None` when it does not exist, since opening it would create it.
///
/// # Errors
/// The [`LedgerError`] when it cannot be found or opened. Only a missing file means no
/// ledger; anything else at its path (a directory, an unreadable file) is opened, so its
/// failure is reported rather than read as "no ledger".
pub(crate) fn open_existing(
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
    git_timeout: Duration,
    busy_timeout: Duration,
) -> Result<Option<Store>, LedgerError> {
    let location =
        paths::ledger_store_within(None, lookup, cwd, git_timeout).map_err(LedgerError::Locate)?;
    if let Err(error) = fs::metadata(&location.path)
        && error.kind() == io::ErrorKind::NotFound
    {
        return Ok(None);
    }
    Store::open_with_busy_timeout(&location.path, busy_timeout)
        .map(Some)
        .map_err(LedgerError::Open)
}
