//! A JSON file named on the command line, read whole within a size cap and parsed strictly.
//!
//! `rollout` (its bands) and `perf` (its baseline samples) share it, and `UniqueMap`, an
//! object whose repeated key is an error rather than a silent overwrite.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::marker::PhantomData;
use std::path::Path;

use serde::de::{self, DeserializeOwned, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};

/// A JSON object of named metrics in which a name given twice does not parse: serde's maps
/// keep the last value, so a repeated key would silently replace the first.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UniqueMap<V>(BTreeMap<String, V>);

impl<V> UniqueMap<V> {
    /// The entries, by name.
    pub(crate) fn into_inner(self) -> BTreeMap<String, V> {
        self.0
    }
}

impl<'de, V: Deserialize<'de>> Deserialize<'de> for UniqueMap<V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(UniqueMapVisitor(PhantomData))
    }
}

/// Reads a [`UniqueMap`] entry by entry.
struct UniqueMapVisitor<V>(PhantomData<V>);

impl<'de, V: Deserialize<'de>> Visitor<'de> for UniqueMapVisitor<V> {
    type Value = UniqueMap<V>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an object of named metrics, each name once")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut entries = BTreeMap::new();
        while let Some((key, value)) = map.next_entry::<String, V>()? {
            match entries.entry(key) {
                Entry::Occupied(entry) => {
                    return Err(de::Error::custom(format_args!(
                        "metric {:?} is given twice",
                        entry.key()
                    )));
                }
                Entry::Vacant(entry) => {
                    entry.insert(value);
                }
            }
        }
        Ok(UniqueMap(entries))
    }
}

/// Why a JSON file could not be read.
#[derive(Debug, thiserror::Error)]
pub enum JsonFileError {
    /// The path is not a regular file (a directory, a FIFO, a device).
    #[error("not a regular file")]
    NotAFile,
    /// The file could not be found, opened or read.
    #[error("reading the file")]
    Read(#[source] io::Error),
    /// The file is larger than the cap.
    #[error("larger than {cap} bytes")]
    TooLarge {
        /// The cap, in bytes.
        cap: u64,
    },
    /// The file is not the JSON expected.
    #[error("parsing the file")]
    Parse(#[source] serde_json::Error),
}

/// Reads `path` (a regular file of at most `cap` bytes) and parses it as `T`.
///
/// # Errors
/// The [`JsonFileError`] when it is not a regular file, cannot be read, is over `cap` or does
/// not parse.
pub fn read<T: DeserializeOwned>(path: &Path, cap: u64) -> Result<T, JsonFileError> {
    // Checked before opening: a FIFO would block, and a directory opens on unix only.
    let metadata = fs::metadata(path).map_err(JsonFileError::Read)?;
    if !metadata.is_file() {
        return Err(JsonFileError::NotAFile);
    }
    let file = File::open(path).map_err(JsonFileError::Read)?;
    let mut bytes = Vec::new();
    // One byte past the cap tells a file that grew since its size was read: too large, never
    // silently truncated.
    file.take(cap.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(JsonFileError::Read)?;
    if u64::try_from(bytes.len()).map_or(true, |len| len > cap) {
        return Err(JsonFileError::TooLarge { cap });
    }
    serde_json::from_slice(&bytes).map_err(JsonFileError::Parse)
}

#[cfg(test)]
mod tests {
    use super::{JsonFileError, read};
    use std::collections::BTreeMap;

    #[test]
    fn a_file_within_the_cap_parses() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("x.json");
        std::fs::write(&path, br#"{"a": 1}"#).expect("writes");

        let value: BTreeMap<String, u32> = read(&path, 8).expect("8 bytes fit");

        assert_eq!(value.get("a"), Some(&1), "parsed");
    }

    #[test]
    fn a_file_over_the_cap_a_directory_and_bad_json_are_refused() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("x.json");
        std::fs::write(&path, br#"{"a": 1}"#).expect("writes");

        assert!(
            matches!(
                read::<BTreeMap<String, u32>>(&path, 7),
                Err(JsonFileError::TooLarge { cap: 7 })
            ),
            "one byte over"
        );
        assert!(
            matches!(
                read::<BTreeMap<String, u32>>(dir.path(), 1024),
                Err(JsonFileError::NotAFile)
            ),
            "a directory is never opened"
        );
        assert!(
            matches!(read::<Vec<u32>>(&path, 1024), Err(JsonFileError::Parse(_))),
            "an object is not a list"
        );
    }
}
