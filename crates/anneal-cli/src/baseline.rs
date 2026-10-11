//! Baseline snapshots: record the findings present at session start, then report only new ones.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::envelope::Finding;

/// Snapshot files larger than this are refused rather than read into memory (H-13).
pub const MAX_SNAPSHOT_BYTES: u64 = 16 * 1024 * 1024;

const FORMAT_VERSION: u32 = 1;

/// The set of finding keys present when the snapshot was taken.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Baseline {
    keys: BTreeSet<String>,
}

/// On-disk form: `{"version": 1, "keys": [...]}`, keys sorted so the file is deterministic.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotFile {
    /// Declared so `deny_unknown_fields` accepts it; [`VersionProbe`] has already checked it.
    #[serde(rename = "version")]
    _version: u32,
    keys: BTreeSet<String>,
}

/// Just the version, read without rejecting unknown fields.
#[derive(Debug, Deserialize)]
struct VersionProbe {
    version: u32,
}

/// The same shape as [`SnapshotFile`], borrowing the keys for writing.
#[derive(Debug, Serialize)]
struct SnapshotRef<'a> {
    version: u32,
    keys: &'a BTreeSet<String>,
}

/// Why a snapshot could not be loaded or saved.
#[derive(Debug, thiserror::Error)]
pub enum BaselineError {
    /// The file could not be read or written.
    #[error("{op} {path}", path = path.display())]
    Io {
        /// `"reading"` or `"writing"`.
        op: &'static str,
        /// The snapshot path.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: io::Error,
    },
    /// The file is larger than [`MAX_SNAPSHOT_BYTES`].
    #[error("{path} is larger than {MAX_SNAPSHOT_BYTES} bytes", path = path.display())]
    TooLarge {
        /// The snapshot path.
        path: PathBuf,
    },
    /// The file is not a snapshot this version understands.
    #[error("{path} is not a valid baseline snapshot", path = path.display())]
    Format {
        /// The snapshot path.
        path: PathBuf,
        /// The parse error.
        #[source]
        source: serde_json::Error,
    },
    /// The file has a format version this build does not know.
    #[error("{path} has snapshot version {found}, expected {FORMAT_VERSION}", path = path.display())]
    Version {
        /// The snapshot path.
        path: PathBuf,
        /// The version in the file.
        found: u32,
    },
}

impl Baseline {
    /// A baseline holding the keys of `findings`. The keys are copied: the baseline owns them.
    pub fn from_findings<'f>(findings: impl IntoIterator<Item = &'f Finding>) -> Self {
        Self {
            keys: findings.into_iter().map(|f| f.key.clone()).collect(),
        }
    }

    /// True when `finding` was already present at snapshot time.
    #[must_use]
    pub fn contains(&self, finding: &Finding) -> bool {
        self.keys.contains(&finding.key)
    }

    /// The findings that were not present at snapshot time, in input order.
    pub fn new_findings<'f>(
        &self,
        findings: impl IntoIterator<Item = &'f Finding>,
    ) -> impl Iterator<Item = &'f Finding> {
        findings.into_iter().filter(|f| !self.contains(f))
    }

    /// Loads the snapshot at `path`. A missing file is `Ok(None)`: no snapshot was taken, so
    /// the caller decides whether every finding counts as new.
    ///
    /// # Errors
    /// [`BaselineError`] when the file cannot be read, is too large, or is not a version-1
    /// snapshot.
    pub fn load(path: &Path) -> Result<Option<Self>, BaselineError> {
        load_capped(path, MAX_SNAPSHOT_BYTES)
    }

    /// Saves the snapshot to `path`, creating its parent directory. Writes and syncs a sibling
    /// temporary file unique to this writer, then renames it over `path`, so a reader never
    /// sees a half-written snapshot; a failed save removes its temporary file.
    ///
    /// # Errors
    /// [`BaselineError::Io`] when the directory or file cannot be written.
    pub fn save(&self, path: &Path) -> Result<(), BaselineError> {
        let io_error = |source| BaselineError::Io {
            op: "writing",
            path: path.to_path_buf(),
            source,
        };
        let snapshot = SnapshotRef {
            version: FORMAT_VERSION,
            keys: &self.keys,
        };
        let mut json = serde_json::to_vec_pretty(&snapshot).map_err(|e| io_error(e.into()))?;
        json.push(b'\n');
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        write_atomically(path, &json).map_err(io_error)
    }
}

/// [`Baseline::load`] with the size limit as a parameter, so tests can exercise it cheaply.
fn load_capped(path: &Path, cap: u64) -> Result<Option<Baseline>, BaselineError> {
    let io_error = |source| BaselineError::Io {
        op: "reading",
        path: path.to_path_buf(),
        source,
    };
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(error)),
    };
    let mut bytes = Vec::new();
    file.take(cap.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if !u64::try_from(bytes.len()).is_ok_and(|len| len <= cap) {
        return Err(BaselineError::TooLarge {
            path: path.to_path_buf(),
        });
    }
    parse(path, &bytes).map(Some)
}

/// Reads the version leniently first, so a snapshot from a newer build is reported as a
/// [`BaselineError::Version`] even when its other fields changed shape.
fn parse(path: &Path, bytes: &[u8]) -> Result<Baseline, BaselineError> {
    let format_error = |source| BaselineError::Format {
        path: path.to_path_buf(),
        source,
    };
    let probe: VersionProbe = serde_json::from_slice(bytes).map_err(format_error)?;
    if probe.version != FORMAT_VERSION {
        return Err(BaselineError::Version {
            path: path.to_path_buf(),
            found: probe.version,
        });
    }
    let snapshot: SnapshotFile = serde_json::from_slice(bytes).map_err(format_error)?;
    Ok(Baseline {
        keys: snapshot.keys,
    })
}

/// Distinguishes concurrent writers' temporary files within one process.
static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

/// How many fresh temporary names a save tries before giving up.
const MAX_TEMPORARY_ATTEMPTS: u32 = 8;

/// `<path>.<pid>.<n>.tmp`, beside `path`, with `n` new on every call in this process.
fn temporary_path(path: &Path) -> PathBuf {
    // Relaxed: the counter only has to hand out distinct numbers; it orders no other memory.
    let n = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{}.{n}.tmp", std::process::id()));
    PathBuf::from(temporary)
}

/// Creates a temporary file that did not exist before (`create_new`), so two writers can never
/// share one; a name that is taken (say, left by a crashed process with a recycled pid) is
/// skipped for the next. `next_name` supplies the candidates (`temporary_path` in production).
fn create_temporary(mut next_name: impl FnMut() -> PathBuf) -> io::Result<(PathBuf, File)> {
    for _ in 0..MAX_TEMPORARY_ATTEMPTS {
        let temporary = next_name();
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!(
            "no free temporary name after {MAX_TEMPORARY_ATTEMPTS} attempts; \
             remove stale temporary files (*.tmp) beside the snapshot"
        ),
    ))
}

fn write_and_sync(mut file: File, bytes: &[u8]) -> io::Result<()> {
    file.write_all(bytes)?;
    file.sync_all()
}

/// Writes `bytes` to a temporary file unique to this writer (`<path>.<pid>.<n>.tmp`), syncs it,
/// and renames it over `path`, so readers see the old file or the new one, never a mix. On
/// failure the temporary file is removed.
fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let (temporary, file) = create_temporary(|| temporary_path(path))?;
    // `write_and_sync` consumes the file, so it is closed before the rename (Windows needs that).
    let written = write_and_sync(file, bytes).and_then(|()| fs::rename(&temporary, path));
    match written {
        Ok(()) => Ok(()),
        Err(error) => match fs::remove_file(&temporary) {
            Ok(()) => Err(error),
            Err(cleanup) if cleanup.kind() == io::ErrorKind::NotFound => Err(error),
            Err(cleanup) => Err(io::Error::new(
                error.kind(),
                format!(
                    "{error}; removing {} also failed: {cleanup}",
                    temporary.display()
                ),
            )),
        },
    }
}

#[cfg(test)]
mod tests;
