//! Baseline snapshots: record the findings present at session start, then report only new ones.

use std::collections::BTreeSet;
use std::fs::{self, File};
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

/// Writes `bytes` to a temporary file unique to this writer (`<path>.<pid>.<n>.tmp`), syncs it,
/// and renames it over `path`, so readers see the old file or the new one, never a mix. On
/// failure the temporary file is removed.
fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    // Relaxed: the counter only has to hand out distinct numbers; it orders no other memory.
    let n = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{}.{n}.tmp", std::process::id()));
    let temporary = PathBuf::from(temporary);
    let written = File::create(&temporary)
        .and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&temporary, path));
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
mod tests {
    use super::{Baseline, BaselineError, load_capped};
    use crate::envelope::Finding;

    const SNAPSHOT: &str = r#"{"version": 1, "keys": ["a"]}"#;

    #[test]
    fn a_snapshot_over_the_cap_is_too_large() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("baseline.json");
        std::fs::write(&path, SNAPSHOT).expect("write");
        let cap = u64::try_from(SNAPSHOT.len()).expect("small") - 1;

        let error = load_capped(&path, cap).expect_err("one byte over the cap");

        assert!(
            matches!(error, BaselineError::TooLarge { .. }),
            "got {error:?}"
        );
    }

    #[test]
    fn a_snapshot_exactly_at_the_cap_loads() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("baseline.json");
        std::fs::write(&path, SNAPSHOT).expect("write");
        let cap = u64::try_from(SNAPSHOT.len()).expect("small");

        let loaded = load_capped(&path, cap).expect("at the cap is allowed");

        assert_eq!(
            loaded,
            Some(Baseline::from_findings(&[finding("a")])),
            "the snapshot loads in full"
        );
    }

    #[test]
    fn a_newer_version_with_a_changed_shape_is_a_version_error() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("baseline.json");
        std::fs::write(&path, r#"{"version": 2, "entries": [{"key": "a"}]}"#).expect("write");

        let error = Baseline::load(&path).expect_err("version 2 is unknown");

        assert!(
            matches!(error, BaselineError::Version { found: 2, .. }),
            "got {error:?}"
        );
    }

    #[test]
    fn saving_leaves_no_temporary_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("baseline.json");

        Baseline::from_findings(&[finding("a")])
            .save(&path)
            .expect("first save");
        Baseline::from_findings(&[finding("b")])
            .save(&path)
            .expect("overwrite");

        let names = file_names(dir.path());
        assert_eq!(names, ["baseline.json"], "only the snapshot remains");
    }

    #[test]
    fn a_failed_save_removes_its_temporary_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        // A non-empty directory where the snapshot should go makes the rename fail.
        let path = dir.path().join("baseline.json");
        std::fs::create_dir(&path).expect("mkdir");
        std::fs::write(path.join("occupant"), "x").expect("write");

        let error = Baseline::from_findings(&[finding("a")])
            .save(&path)
            .expect_err("cannot rename over a directory");

        assert!(
            matches!(error, BaselineError::Io { op: "writing", .. }),
            "got {error:?}"
        );
        assert_eq!(file_names(dir.path()), ["baseline.json"], "no temp left");
    }

    fn file_names(dir: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .expect("read dir")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    fn finding(key: &str) -> Finding {
        Finding::new(key, format!("message for {key}"))
    }

    #[test]
    fn only_findings_absent_from_the_snapshot_are_new() {
        let baseline = Baseline::from_findings(&[finding("a"), finding("b")]);
        let later = [finding("b"), finding("c"), finding("a"), finding("d")];

        let new: Vec<&str> = baseline
            .new_findings(&later)
            .map(|f| f.key.as_str())
            .collect();

        assert_eq!(new, ["c", "d"], "a and b were already there");
    }

    #[test]
    fn a_saved_snapshot_loads_back_equal() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("nested/baseline.json");
        let baseline = Baseline::from_findings(&[finding("z"), finding("a")]);

        baseline.save(&path).expect("save");
        let loaded = Baseline::load(&path).expect("load");

        assert_eq!(loaded, Some(baseline), "round trip keeps the keys");
    }

    #[test]
    fn a_missing_snapshot_loads_as_none() {
        let dir = tempfile::tempdir().expect("temp dir");
        let loaded = Baseline::load(&dir.path().join("absent.json")).expect("load");
        assert_eq!(loaded, None, "no snapshot taken yet");
    }

    #[test]
    fn an_unknown_version_is_refused() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("baseline.json");
        std::fs::write(&path, r#"{"version": 2, "keys": []}"#).expect("write");

        let error = Baseline::load(&path).expect_err("version 2 is unknown");

        assert!(
            matches!(error, BaselineError::Version { found: 2, .. }),
            "got {error:?}"
        );
    }

    #[test]
    fn garbage_is_a_format_error() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("baseline.json");
        std::fs::write(&path, "not json").expect("write");

        let error = Baseline::load(&path).expect_err("not a snapshot");

        assert!(
            matches!(error, BaselineError::Format { .. }),
            "got {error:?}"
        );
    }
}
