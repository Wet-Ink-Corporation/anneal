//! Finding skill directories under the given paths and reading their `SKILL.md`.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::lint::skills::LintError;

/// The file every skill directory holds.
pub(crate) const SKILL_FILE: &str = "SKILL.md";

/// Files read whole (`SKILL.md`, hooks metadata, the index) larger than this are not read past
/// it (H-13). Anything near it already breaks the 8 KB budget.
pub(crate) const MAX_READ_BYTES: u64 = 1024 * 1024;

/// A directory holding a `SKILL.md`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SkillDir {
    /// The directory, as reached from the command-line path.
    pub dir: PathBuf,
    /// The directory's own name (lossy if not UTF-8), which `name` must equal.
    pub name: String,
}

impl SkillDir {
    /// The `SKILL.md` path.
    pub(crate) fn skill_file(&self) -> PathBuf {
        self.dir.join(SKILL_FILE)
    }
}

/// The skill directories under `paths`, sorted and without duplicates. Each path is either a
/// skill directory itself or a directory of skills; subdirectories and files without a
/// `SKILL.md` are ignored.
///
/// # Errors
/// [`LintError::MissingPath`] when a path does not exist; [`LintError::WrongKind`] when it is
/// not a directory; [`LintError::Io`] when a directory cannot be listed.
pub(crate) fn discover(paths: &[PathBuf]) -> Result<Vec<SkillDir>, LintError> {
    let mut skills = Vec::new();
    for path in paths {
        if !path.exists() {
            return Err(LintError::MissingPath { path: path.clone() });
        }
        if path.join(SKILL_FILE).is_file() {
            skills.push(skill_dir(path.clone()));
            continue;
        }
        if !path.is_dir() {
            return Err(LintError::WrongKind {
                path: path.clone(),
                expected: "a skill directory or a directory of skills",
            });
        }
        let entries = fs::read_dir(path).map_err(|source| LintError::io(path, source))?;
        for entry in entries {
            let entry = entry.map_err(|source| LintError::io(path, source))?;
            let dir = entry.path();
            let is_dir = entry
                .file_type()
                .map_err(|source| LintError::io(&dir, source))?
                .is_dir();
            if is_dir && dir.join(SKILL_FILE).is_file() {
                skills.push(skill_dir(dir));
            }
        }
    }
    skills.sort();
    skills.dedup();
    Ok(skills)
}

/// A skill directory named after its last path component. A path with none (`.`, `..`) is
/// resolved first; if that fails too the name is empty, which the `name` check then reports
/// as a mismatch rather than hiding.
fn skill_dir(dir: PathBuf) -> SkillDir {
    let resolved = dir.file_name().map(PathBuf::from).or_else(|| {
        fs::canonicalize(&dir)
            .ok()
            .and_then(|path| path.file_name().map(PathBuf::from))
    });
    let name = resolved.map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    SkillDir { dir, name }
}

/// A file read up to [`MAX_READ_BYTES`]: its full size and, when within the cap, its bytes.
#[derive(Debug)]
pub(crate) struct Capped {
    /// The file's size on disk.
    pub size: u64,
    /// The contents, or `None` when the file is larger than the cap.
    pub bytes: Option<Vec<u8>>,
}

/// Reads `path` up to [`MAX_READ_BYTES`].
///
/// # Errors
/// [`LintError::Io`] when the file cannot be opened or read.
pub(crate) fn read_capped(path: &Path) -> Result<Capped, LintError> {
    let file = File::open(path).map_err(|source| LintError::io(path, source))?;
    let size = file
        .metadata()
        .map_err(|source| LintError::io(path, source))?
        .len();
    if size > MAX_READ_BYTES {
        return Ok(Capped { size, bytes: None });
    }
    let mut bytes = Vec::new();
    file.take(MAX_READ_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|source| LintError::io(path, source))?;
    Ok(Capped {
        size,
        bytes: Some(bytes),
    })
}

/// What a directory entry is, as `DirEntry::file_type` reports it: symlinks are never followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum EntryKind {
    /// A regular file.
    File,
    /// A directory.
    Dir,
    /// A symbolic link, whatever it points at (or nothing).
    Symlink,
    /// Anything else: a FIFO, socket or device.
    Other,
}

impl EntryKind {
    fn of(file_type: fs::FileType) -> Self {
        if file_type.is_symlink() {
            Self::Symlink
        } else if file_type.is_dir() {
            Self::Dir
        } else if file_type.is_file() {
            Self::File
        } else {
            Self::Other
        }
    }

    /// A stable byte for hashing.
    pub(crate) const fn tag(self) -> u8 {
        match self {
            Self::File => 0,
            Self::Dir => 1,
            Self::Symlink => 2,
            Self::Other => 3,
        }
    }
}

/// The entries of `dir` (names and kinds), sorted by name. A missing directory has no
/// entries. Symlinks are reported as [`EntryKind::Symlink`] and never followed.
///
/// # Errors
/// [`LintError::Io`] when `dir` exists but cannot be listed.
pub(crate) fn list_dir(dir: &Path) -> Result<Vec<(String, EntryKind)>, LintError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(LintError::io(dir, source)),
    };
    let mut listed = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| LintError::io(dir, source))?;
        let file_type = entry
            .file_type()
            .map_err(|source| LintError::io(dir, source))?;
        listed.push((
            entry.file_name().to_string_lossy().into_owned(),
            EntryKind::of(file_type),
        ));
    }
    listed.sort();
    Ok(listed)
}
