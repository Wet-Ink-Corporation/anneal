//! The content-hash snapshot (wshobson's pattern): per-skill findings keyed by a SHA-256 of the
//! skill's files and of every input the per-skill checks read. An unchanged skill is not
//! re-checked; its stored findings are replayed, so a cached run reports exactly what a fresh
//! one would.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use serde::de;

use crate::baseline::{BaselineError, read_snapshot_bytes, write_snapshot_json};
use crate::lint::finding::LintFinding;
use crate::lint::skills::LintError;
use crate::lint::skills::discover::{EntryKind, MAX_READ_BYTES, SkillDir, list_dir};

const FORMAT_VERSION: u32 = 1;
/// How deep the hash walks into a skill directory; deeper entries are hashed by name only.
const MAX_DEPTH: usize = 4;

/// Stored per-skill findings, keyed by skill directory.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Cache {
    version: u32,
    skills: BTreeMap<String, Entry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Entry {
    hash: String,
    findings: Vec<LintFinding>,
}

impl Cache {
    /// Loads the snapshot at `path`. A missing file is an empty cache. So is a JSON object with
    /// a numeric `version` that is another version or another shape: a snapshot this build
    /// cannot replay is stale, every skill is checked afresh, and `save` replaces it.
    ///
    /// # Errors
    /// [`LintError::WrongKind`] when `path` is a directory. [`LintError::Cache`] when the file
    /// cannot be read, or is not a JSON object with a numeric `version`: it may be some other
    /// file, so it is refused rather than overwritten.
    pub(crate) fn load(path: &Path) -> Result<Self, LintError> {
        if path.is_dir() {
            return Err(LintError::WrongKind {
                path: path.to_path_buf(),
                expected: "a file",
            });
        }
        let Some(bytes) = read_snapshot_bytes(path)? else {
            return Ok(Self::empty());
        };
        let format_error = |source| BaselineError::Format {
            path: path.to_path_buf(),
            source,
        };
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(format_error)?;
        let version = value
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| format_error(de::Error::custom("not a versioned lint snapshot")))?;
        if version != u64::from(FORMAT_VERSION) {
            return Ok(Self::empty());
        }
        // Our version but another shape: written by an older build of the same format.
        Ok(serde_json::from_value(value).unwrap_or_else(|_stale| Self::empty()))
    }

    fn empty() -> Self {
        Self {
            version: FORMAT_VERSION,
            skills: BTreeMap::new(),
        }
    }

    /// The stored findings for `dir` when its hash is still `hash`.
    pub(crate) fn lookup(&self, dir: &str, hash: &str) -> Option<&[LintFinding]> {
        self.skills
            .get(dir)
            .filter(|entry| entry.hash == hash)
            .map(|entry| entry.findings.as_slice())
    }

    /// Records `findings` for `dir` at `hash`.
    pub(crate) fn store(&mut self, dir: String, hash: String, findings: Vec<LintFinding>) {
        self.skills.insert(dir, Entry { hash, findings });
    }

    /// Drops entries for skills not seen in this run.
    pub(crate) fn retain(&mut self, seen: &BTreeSet<String>) {
        self.skills.retain(|dir, _| seen.contains(dir));
    }

    /// Saves the snapshot atomically.
    ///
    /// # Errors
    /// [`LintError::Cache`] when it cannot be written.
    pub(crate) fn save(&self, path: &Path) -> Result<(), LintError> {
        write_snapshot_json(path, self).map_err(LintError::from)
    }
}

/// The source of every module whose code decides per-skill findings. Hashing it means an edited
/// check invalidates the snapshot by itself; there is no version number to forget to bump.
const RULE_SOURCES: [&str; 8] = [
    include_str!("checker.rs"),
    include_str!("checks_body.rs"),
    include_str!("checks_prose.rs"),
    include_str!("checks_set.rs"),
    include_str!("checks_spec.rs"),
    include_str!("document.rs"),
    include_str!("frontmatter.rs"),
    include_str!("../finding.rs"),
];

/// The hex SHA-256 of everything the per-skill checks read: the rule sources, the hook ids,
/// and each entry under the skill directory (see `hash_dir`).
///
/// # Errors
/// [`LintError::Io`] when a file cannot be read.
pub(crate) fn content_hash(
    skill: &SkillDir,
    hook_ids: &BTreeSet<String>,
) -> Result<String, LintError> {
    hash_with_rules(&RULE_SOURCES, skill, hook_ids)
}

/// [`content_hash`] with the rule sources given, so a test can vary them.
fn hash_with_rules(
    rules: &[&str],
    skill: &SkillDir,
    hook_ids: &BTreeSet<String>,
) -> Result<String, LintError> {
    let mut hasher = Sha256::new();
    hasher.update(FORMAT_VERSION.to_le_bytes());
    for source in rules {
        hash_bytes(&mut hasher, source.as_bytes());
    }
    hash_bytes(&mut hasher, env!("CARGO_PKG_VERSION").as_bytes());
    for id in hook_ids {
        hash_bytes(&mut hasher, id.as_bytes());
    }
    hash_dir(&mut hasher, &skill.dir, "", 0)?;
    let hex = hasher
        .finalize()
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
        .collect();
    Ok(hex)
}

/// Hashes each entry of `dir` in name order: its relative path and kind, then a regular file's
/// contents, a symlink's target (never followed), or a directory's entries. FIFOs, sockets and
/// devices contribute their name and kind only, so nothing here can block or read without end.
fn hash_dir(hasher: &mut Sha256, dir: &Path, prefix: &str, depth: usize) -> Result<(), LintError> {
    for (name, kind) in list_dir(dir)? {
        let relative = format!("{prefix}{name}");
        hash_bytes(hasher, relative.as_bytes());
        hasher.update([kind.tag()]);
        let path = dir.join(&name);
        match kind {
            EntryKind::File => hash_file(hasher, &path)?,
            EntryKind::Symlink => {
                let target = fs::read_link(&path).map_err(|source| LintError::io(&path, source))?;
                hash_bytes(hasher, target.to_string_lossy().as_bytes());
            }
            EntryKind::Dir if depth < MAX_DEPTH => {
                hash_dir(
                    hasher,
                    &path,
                    &format!("{relative}/"),
                    depth.saturating_add(1),
                )?;
            }
            EntryKind::Dir | EntryKind::Other => {}
        }
    }
    Ok(())
}

/// `bytes`, prefixed by their length so adjacent inputs cannot run together.
fn hash_bytes(hasher: &mut Sha256, bytes: &[u8]) {
    // A slice length always fits in u64 on supported targets; saturating keeps it total.
    hasher.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_le_bytes());
    hasher.update(bytes);
}

/// A regular file's size, then its first [`MAX_READ_BYTES`] bytes, length-prefixed.
fn hash_file(hasher: &mut Sha256, path: &Path) -> Result<(), LintError> {
    let io_error = |source| LintError::io(path, source);
    let file = File::open(path).map_err(io_error)?;
    let size = file.metadata().map_err(io_error)?.len();
    let mut bytes = Vec::new();
    file.take(MAX_READ_BYTES)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    hasher.update(size.to_le_bytes());
    hash_bytes(hasher, &bytes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Cache, RULE_SOURCES, content_hash, hash_with_rules};
    use crate::baseline::BaselineError;
    use crate::lint::skills::LintError;
    use crate::lint::skills::discover::SkillDir;
    use std::collections::BTreeSet;

    fn skill(dir: &std::path::Path) -> SkillDir {
        SkillDir {
            dir: dir.to_path_buf(),
            name: "plan".to_owned(),
        }
    }

    #[test]
    fn the_hash_changes_with_any_file_and_with_the_hook_ids() {
        let root = tempfile::tempdir().expect("temp dir");
        std::fs::write(root.path().join("SKILL.md"), "a").expect("write");
        let none = BTreeSet::new();
        let first = content_hash(&skill(root.path()), &none).expect("hash");

        assert_eq!(
            first,
            content_hash(&skill(root.path()), &none).expect("hash"),
            "stable"
        );
        let hooks: BTreeSet<String> = ["guard".to_owned()].into();
        assert_ne!(
            first,
            content_hash(&skill(root.path()), &hooks).expect("hash"),
            "hook ids count"
        );
        std::fs::create_dir(root.path().join("references")).expect("mkdir");
        std::fs::write(root.path().join("references/x.md"), "b").expect("write");
        assert_ne!(
            first,
            content_hash(&skill(root.path()), &none).expect("hash"),
            "new file counts"
        );
    }

    #[test]
    fn file_boundaries_are_part_of_the_hash() {
        // Without each file's length, {a: "", b: "Q"} and {a: "b\0\0Q"} hash the same bytes.
        let split = tempfile::tempdir().expect("temp dir");
        std::fs::write(split.path().join("a"), "").expect("write");
        std::fs::write(split.path().join("b"), "Q").expect("write");
        let joined = tempfile::tempdir().expect("temp dir");
        std::fs::write(joined.path().join("a"), "b\0\0Q").expect("write");
        let none = BTreeSet::new();

        assert_ne!(
            content_hash(&skill(split.path()), &none).expect("hash"),
            content_hash(&skill(joined.path()), &none).expect("hash"),
            "two files never hash like one"
        );
    }

    #[test]
    fn the_rule_sources_are_part_of_the_hash() {
        let root = tempfile::tempdir().expect("temp dir");
        std::fs::write(root.path().join("SKILL.md"), "a").expect("write");
        let none = BTreeSet::new();
        let skill = skill(root.path());

        assert_ne!(
            hash_with_rules(&["fn check() {}"], &skill, &none).expect("hash"),
            hash_with_rules(&["fn check() { changed }"], &skill, &none).expect("hash"),
            "editing a check invalidates every cached entry"
        );
        assert!(
            RULE_SOURCES.len() >= 8,
            "checker, checks_*, document, frontmatter"
        );
    }

    fn write(dir: &tempfile::TempDir, text: &str) -> std::path::PathBuf {
        let path = dir.path().join("cache.json");
        std::fs::write(&path, text).expect("write");
        path
    }

    #[test]
    fn a_snapshot_from_another_version_or_shape_loads_as_empty() {
        let root = tempfile::tempdir().expect("temp dir");
        for text in [
            r#"{"version": 99, "skills": []}"#,
            r#"{"version": 1, "skills": 5}"#,
        ] {
            let cache = Cache::load(&write(&root, text)).expect("stale, not an error");
            assert_eq!(
                cache.lookup("any", "hash"),
                None,
                "{text}: nothing replayed"
            );
        }
    }

    #[test]
    fn a_file_that_is_not_a_versioned_snapshot_is_a_format_error() {
        let root = tempfile::tempdir().expect("temp dir");
        for text in [
            "not json",
            "[1, 2]",
            r#"{"skills": {}}"#,
            r#"{"version": "1"}"#,
        ] {
            let result = Cache::load(&write(&root, text));
            assert!(
                matches!(result, Err(LintError::Cache(BaselineError::Format { .. }))),
                "{text}: refused rather than overwritten: {result:?}"
            );
        }
    }
}
