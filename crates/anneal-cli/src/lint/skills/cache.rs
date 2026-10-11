//! The content-hash snapshot (wshobson's pattern): per-skill findings keyed by a SHA-256 of the
//! skill's files and of every input the per-skill checks read. An unchanged skill is not
//! re-checked; its stored findings are replayed, so a cached run reports exactly what a fresh
//! one would.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::baseline::{read_snapshot_bytes, write_snapshot_json};
use crate::lint::finding::LintFinding;
use crate::lint::skills::LintError;
use crate::lint::skills::discover::SkillDir;

/// Bumped whenever a check changes, so snapshots from older rules are not replayed.
const RULES_VERSION: u32 = 1;
const FORMAT_VERSION: u32 = 1;
/// How deep the hash walks into a skill directory; deeper entries are hashed by name only.
const MAX_DEPTH: usize = 4;

/// Stored per-skill findings, keyed by skill directory.
#[derive(Debug, Default, Serialize, Deserialize)]
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
    /// Loads the snapshot at `path`. A missing file, or one written by another format or rule
    /// version, is an empty cache: every skill is checked afresh, which is always correct.
    ///
    /// # Errors
    /// [`LintError::Cache`] when the file exists but cannot be read.
    pub(crate) fn load(path: &Path) -> Result<Self, LintError> {
        let Some(bytes) = read_snapshot_bytes(path)? else {
            return Ok(Self::empty());
        };
        // A snapshot that does not parse is stale, not an error: it is rebuilt on save.
        Ok(serde_json::from_slice::<Self>(&bytes)
            .ok()
            .filter(|cache| cache.version == FORMAT_VERSION)
            .unwrap_or_else(Self::empty))
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

/// The hex SHA-256 of everything the per-skill checks read: the rules version, the hook ids,
/// and each file under the skill directory (relative path and contents, in sorted order).
///
/// # Errors
/// [`LintError::Io`] when a file cannot be read.
pub(crate) fn content_hash(
    skill: &SkillDir,
    hook_ids: &BTreeSet<String>,
) -> Result<String, LintError> {
    let mut hasher = Sha256::new();
    hasher.update(RULES_VERSION.to_le_bytes());
    hasher.update(env!("CARGO_PKG_VERSION"));
    for id in hook_ids {
        hasher.update(id.as_bytes());
        hasher.update([0]);
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

fn hash_dir(hasher: &mut Sha256, dir: &Path, prefix: &str, depth: usize) -> Result<(), LintError> {
    for (name, is_dir) in super::discover::list_dir(dir)? {
        let relative = format!("{prefix}{name}");
        hasher.update(relative.as_bytes());
        hasher.update([0, u8::from(is_dir)]);
        let path = dir.join(&name);
        if is_dir && depth < MAX_DEPTH {
            hash_dir(
                hasher,
                &path,
                &format!("{relative}/"),
                depth.saturating_add(1),
            )?;
        } else if !is_dir {
            hash_file(hasher, &path)?;
        }
    }
    Ok(())
}

fn hash_file(hasher: &mut Sha256, path: &Path) -> Result<(), LintError> {
    let mut file = File::open(path).map_err(|source| LintError::io(path, source))?;
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| LintError::io(path, source))?;
        let Some(chunk) = buffer.get(..read).filter(|c| !c.is_empty()) else {
            return Ok(());
        };
        hasher.update(chunk);
    }
}

#[cfg(test)]
mod tests {
    use super::{Cache, content_hash};
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
    fn an_unparseable_snapshot_loads_as_empty() {
        let root = tempfile::tempdir().expect("temp dir");
        let path = root.path().join("cache.json");
        std::fs::write(&path, "not json").expect("write");

        let cache = Cache::load(&path).expect("stale snapshots are not errors");

        assert_eq!(cache.lookup("any", "hash"), None, "nothing replayed");
    }
}
