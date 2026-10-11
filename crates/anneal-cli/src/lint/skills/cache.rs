//! The content-hash snapshot (wshobson's pattern): per-skill findings keyed by a SHA-256 of
//! every input the per-skill checks consume. An unchanged skill is not re-checked; its stored
//! findings are replayed, so a cached run reports exactly what a fresh one would.
//!
//! The inputs are the `SKILL.md` bytes as loaded (through any symlink), the skill's name, the
//! [`Surroundings`] (`scripts/` present, the `references/` listing), the hook ids, the anneal
//! version and the source of the checks themselves. Nothing else on disk is read by a per-skill
//! check, so nothing else is hashed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::de;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::baseline::{BaselineError, read_snapshot_bytes, write_snapshot_json};
use crate::lint::finding::LintFinding;
use crate::lint::skills::LintError;
use crate::lint::skills::discover::Surroundings;
use crate::lint::skills::document::Skill;

/// Identifies a lint-skills snapshot, so `--cache` never mistakes another JSON file (hooks
/// metadata, a baseline) for a stale snapshot and overwrites it.
const KIND: &str = "anneal-lint-skills";
/// 2: added `kind`; per-skill inputs instead of a directory walk.
const FORMAT_VERSION: u32 = 2;

// Lengths are hashed as u64; this keeps `u64::try_from(usize)` below total.
const _: () = assert!(usize::BITS <= 64, "lengths are hashed as u64");

/// Stored per-skill findings, keyed by skill directory.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Cache {
    kind: String,
    version: u32,
    skills: BTreeMap<String, Entry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    hash: String,
    findings: Vec<LintFinding>,
}

impl Cache {
    /// Loads the snapshot at `path`. A missing file is an empty cache. So is one of ours (its
    /// `kind` is `anneal-lint-skills`) written in another format version: it is stale, every
    /// skill is checked afresh, and `save` replaces it.
    ///
    /// # Errors
    /// [`LintError::WrongKind`] when `path` is a directory. [`LintError::Cache`] when the file
    /// cannot be read, does not carry our `kind` (it is some other file, so it is refused
    /// rather than overwritten), or carries it with the current version but does not
    /// deserialize.
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
        let not_ours = || format_error(de::Error::custom("not an anneal lint-skills snapshot"));
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(format_error)?;
        if value.get("kind").and_then(serde_json::Value::as_str) != Some(KIND) {
            return Err(not_ours().into());
        }
        let version = value
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(not_ours)?;
        if version != u64::from(FORMAT_VERSION) {
            return Ok(Self::empty());
        }
        Ok(serde_json::from_value(value).map_err(format_error)?)
    }

    fn empty() -> Self {
        Self {
            kind: KIND.to_owned(),
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

/// Builds [`RULE_SOURCES`] from paths relative to this file, keeping each path beside the text
/// `include_str!` reads from it.
macro_rules! rule_sources {
    ($($path:literal),* $(,)?) => {
        /// Every module whose code decides per-skill findings, as (path relative to
        /// `src/lint/skills/`, source). Hashing the source means an edited check invalidates
        /// the snapshot by itself.
        const RULE_SOURCES: &[(&str, &str)] = &[$(($path, include_str!($path))),*];
    };
}

rule_sources!(
    "checker.rs",
    "checks_body.rs",
    "checks_prose.rs",
    "checks_set.rs",
    "checks_spec.rs",
    "discover.rs",
    "document.rs",
    "frontmatter.rs",
    "../finding.rs",
    "../skills.rs",
    "../../command.rs",
);

/// The hex SHA-256 of everything the per-skill checks consume for `skill`.
pub(crate) fn content_hash(
    skill: &Skill<'_>,
    around: &Surroundings,
    hook_ids: &BTreeSet<String>,
) -> String {
    hash_with_rules(RULE_SOURCES, skill, around, hook_ids)
}

/// [`content_hash`] with the rule sources given, so a test can vary them.
fn hash_with_rules(
    rules: &[(&str, &str)],
    skill: &Skill<'_>,
    around: &Surroundings,
    hook_ids: &BTreeSet<String>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(FORMAT_VERSION.to_le_bytes());
    for (path, source) in rules {
        hash_bytes(&mut hasher, path.as_bytes());
        hash_bytes(&mut hasher, source.as_bytes());
    }
    hash_bytes(&mut hasher, env!("CARGO_PKG_VERSION").as_bytes());
    hash_count(&mut hasher, hook_ids.len());
    for id in hook_ids {
        hash_bytes(&mut hasher, id.as_bytes());
    }
    hash_bytes(&mut hasher, skill.dir.name.as_bytes());
    hasher.update(skill.size.to_le_bytes());
    hash_bytes(&mut hasher, skill.doc.source.as_bytes());
    hasher.update([u8::from(around.has_scripts)]);
    hash_count(&mut hasher, around.references.len());
    for (name, kind) in &around.references {
        hash_bytes(&mut hasher, name.as_encoded_bytes());
        hasher.update([kind.tag()]);
    }
    hasher
        .finalize()
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
        .collect()
}

/// A count, as u64, so lists of different lengths cannot run together.
fn hash_count(hasher: &mut Sha256, count: usize) {
    // Total by the `usize::BITS <= 64` assertion above.
    hasher.update(u64::try_from(count).unwrap_or(u64::MAX).to_le_bytes());
}

/// `bytes`, prefixed by their length so adjacent inputs cannot run together.
fn hash_bytes(hasher: &mut Sha256, bytes: &[u8]) {
    hash_count(hasher, bytes.len());
    hasher.update(bytes);
}

#[cfg(test)]
mod tests;
