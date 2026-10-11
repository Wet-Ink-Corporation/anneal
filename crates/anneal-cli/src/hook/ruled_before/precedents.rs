//! The rulings D6 offers as options: the `.out-of-scope/` store and accepted weigh-in decision
//! atoms.
//!
//! [`gather`] reads, under a repository root, every ruling in [`rulings::STORE_DIR`] through
//! `rulings::list_within` (an invalid one, or a store over the bounds below, fails the whole
//! list; a symlink is never followed) and then the weigh-in decision
//! atoms in `.kb/_intake/decisions/*.md`, from their frontmatter only: `weighin_item` (the
//! `wi-` id), `status` (only `accepted` counts) and `title` (else `question`) as the summary.
//! An atom that does not read is skipped: the atoms are weigh-in's files, not anneal's.
//!
//! Bounds (H-13): the ruling store may hold at most [`MAX_DIR_ENTRIES`] entries and
//! [`MAX_ATOM_FILES`] rulings; at most [`MAX_DIR_ENTRIES`] entries of the atoms directory are
//! looked at and [`MAX_ATOM_FILES`] atoms read, each only its first [`MAX_ATOM_BYTES`]; at most
//! [`MAX_PRECEDENTS`] precedents are offered, rulings first, then atoms, each in id order;
//! summaries are cut to [`MAX_SUMMARY_BYTES`]. An id must be a decide option word (1 to 64 of
//! `A-Z a-z 0-9 _ . : -`) and not `none`, which stands for "no ruling"; an atom a ruling
//! already cites as its `weigh_in_ref`, or whose id is taken, is not offered again.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::decide::client::MAX_OPTIONS;
use crate::decide::id::is_word;
use crate::lint::skills::frontmatter;
use crate::rulings::{self, ListLimits, RulingError};

/// The atoms directory, relative to the repository root.
const ATOMS_DIR: [&str; 3] = [".kb", "_intake", "decisions"];

/// The option that means "no ruling applies".
pub(crate) const NONE: &str = "none";

/// Most precedents offered: a choice has at most [`MAX_OPTIONS`] options, one of them `none`.
pub(crate) const MAX_PRECEDENTS: usize = MAX_OPTIONS - 1;

/// Most bytes of a summary sent to Clef or injected.
pub(crate) const MAX_SUMMARY_BYTES: usize = 120;

/// Most entries of the ruling store or the atoms directory looked at.
const MAX_DIR_ENTRIES: usize = 4096;

/// Most rulings, and most atoms, read.
const MAX_ATOM_FILES: usize = 256;

/// Most bytes read of each atom: its frontmatter must close within them.
const MAX_ATOM_BYTES: u64 = 16 * 1024;

/// The status an atom must have to count as a ruling.
const ACCEPTED: &str = "accepted";

/// The prefix of a weigh-in item id.
const WEIGH_IN_PREFIX: &str = "wi-";

/// A ruling D6 can offer: its id (the option) and a one-line summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Precedent {
    id: String,
    summary: String,
}

impl Precedent {
    /// The option id: a ruling's `id`, or an atom's `wi-` id.
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    /// One line saying what was ruled.
    pub(crate) fn summary(&self) -> &str {
        &self.summary
    }
}

/// Why the precedents could not be gathered.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PrecedentError {
    /// The ruling store has an invalid ruling, or could not be read.
    #[error(transparent)]
    Rulings(#[from] RulingError),
    /// The atoms directory could not be listed.
    #[error("{path}: reading failed", path = path.display())]
    Atoms {
        /// The directory.
        path: PathBuf,
        /// The failure.
        #[source]
        source: io::Error,
    },
}

/// The precedents under the repository root `root`; see the module documentation.
///
/// # Errors
/// [`PrecedentError::Rulings`] for a ruling store that does not read,
/// [`PrecedentError::Atoms`] for an atoms directory that cannot be listed.
pub(crate) fn gather(root: &Path) -> Result<Vec<Precedent>, PrecedentError> {
    let limits = ListLimits {
        max_entries: MAX_DIR_ENTRIES,
        max_files: MAX_ATOM_FILES,
    };
    let rulings = rulings::list_within(&root.join(rulings::STORE_DIR), limits)?;
    let cited: BTreeSet<&str> = rulings.iter().filter_map(|r| r.weigh_in_ref()).collect();
    // `rulings::list` refuses duplicate ids, so only the atoms can repeat one.
    let mut precedents: Vec<Precedent> = rulings
        .iter()
        .filter(|ruling| offerable(ruling.id()))
        .map(|ruling| Precedent {
            id: ruling.id().to_owned(),
            summary: one_line(ruling.summary()),
        })
        .collect();
    let atoms_dir = ATOMS_DIR.iter().fold(root.to_path_buf(), |p, c| p.join(c));
    let mut atoms = accepted_atoms(&atoms_dir)?;
    atoms.sort_by(|a, b| a.id.cmp(&b.id));
    for atom in atoms {
        let fresh = precedents.iter().all(|taken| taken.id != atom.id);
        if fresh && !cited.contains(atom.id.as_str()) {
            precedents.push(atom);
        }
    }
    precedents.truncate(MAX_PRECEDENTS);
    Ok(precedents)
}

/// Whether `id` can be a D6 option.
fn offerable(id: &str) -> bool {
    is_word(id) && id != NONE
}

/// `text` with its whitespace collapsed, cut to [`MAX_SUMMARY_BYTES`] on a char boundary.
fn one_line(text: &str) -> String {
    let mut line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    line.truncate(line.floor_char_boundary(MAX_SUMMARY_BYTES));
    line
}

/// The accepted atoms in `dir`, in file-name order; a missing directory has none.
fn accepted_atoms(dir: &Path) -> Result<Vec<Precedent>, PrecedentError> {
    let listing_error = |source| PrecedentError::Atoms {
        path: dir.to_path_buf(),
        source,
    };
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(listing_error(error)),
    };
    let mut files = Vec::new();
    for entry in entries.take(MAX_DIR_ENTRIES) {
        let entry = entry.map_err(listing_error)?;
        let markdown = Path::new(&entry.file_name())
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"));
        // `file_type` does not follow symlinks, so only regular files are opened.
        if markdown && entry.file_type().is_ok_and(|kind| kind.is_file()) {
            files.push(entry.path());
        }
    }
    files.sort();
    files.truncate(MAX_ATOM_FILES);
    Ok(files.iter().filter_map(|file| read_atom(file)).collect())
}

/// The precedent in the atom at `file`, if it reads and is accepted.
fn read_atom(file: &Path) -> Option<Precedent> {
    let mut bytes = Vec::new();
    // An atom that cannot be opened or read is skipped, like one that does not parse.
    File::open(file)
        .and_then(|opened| opened.take(MAX_ATOM_BYTES).read_to_end(&mut bytes))
        .ok()?;
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        // The cap can cut a character; the frontmatter before it is still whole.
        Err(error) => std::str::from_utf8(bytes.get(..error.valid_up_to())?).ok()?,
    };
    let doc = frontmatter::parse(text).ok()?;
    let id = doc.value("weighin_item");
    let summary = [doc.value("title"), doc.value("question")]
        .into_iter()
        .find(|value| !value.trim().is_empty())?;
    let valid = doc.value("status") == ACCEPTED && id.starts_with(WEIGH_IN_PREFIX) && offerable(id);
    valid.then(|| Precedent {
        id: id.to_owned(),
        summary: one_line(summary),
    })
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;
    use std::path::Path;

    use super::{
        MAX_ATOM_FILES, MAX_PRECEDENTS, MAX_SUMMARY_BYTES, Precedent, PrecedentError, gather,
    };
    use crate::rulings::RulingError;

    type TestResult = Result<(), Box<dyn Error>>;

    fn write(root: &Path, parts: &[&str], text: &str) -> TestResult {
        let path = parts
            .iter()
            .fold(root.to_path_buf(), |path, part| path.join(part));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, text)?;
        Ok(())
    }

    fn ruling(root: &Path, id: &str, extra: &str) -> TestResult {
        write(
            root,
            &[".out-of-scope", &format!("{id}.md")],
            &format!("---\nid: {id}\nruled_at: 2026-10-11\nsummary: Ruling {id}.\n{extra}---\n"),
        )
    }

    fn atom(root: &Path, item: &str, status: &str) -> TestResult {
        write(
            root,
            &[
                ".kb",
                "_intake",
                "decisions",
                &format!("{item}-some-slug.md"),
            ],
            &format!(
                "---\nid: \"kb-decision-{item}\"\ntitle: \"Which licence: MIT OR Apache-2.0\"\n\
                 kind: decision\nstatus: {status}\nsummary: \"In the context of \\\"x\\\", we \
                 decided.\"\ndepends_on: []\nphase: null\nweighin_item: \"{item}\"\n\
                 question: \"Which licence should anneal ship under?\"\n---\n\n# Body\n"
            ),
        )
    }

    fn ids(root: &Path) -> Result<Vec<String>, PrecedentError> {
        Ok(gather(root)?
            .iter()
            .map(|precedent| precedent.id().to_owned())
            .collect())
    }

    #[test]
    fn an_accepted_weigh_in_atom_is_offered_with_its_wi_id_and_title() -> TestResult {
        let root = tempfile::tempdir()?;
        atom(root.path(), "wi-a53271", "accepted")?;

        let found = gather(root.path())?;

        let [precedent] = found.as_slice() else {
            panic!("one precedent: {found:?}");
        };
        assert_eq!(precedent.id(), "wi-a53271");
        assert_eq!(precedent.summary(), "Which licence: MIT OR Apache-2.0");
        Ok(())
    }

    #[test]
    fn atoms_not_accepted_malformed_or_without_a_wi_id_are_skipped() -> TestResult {
        let root = tempfile::tempdir()?;
        atom(root.path(), "wi-000001", "proposed")?;
        atom(root.path(), "wi-000002", "accepted")?;
        atom(root.path(), "not-wi", "accepted")?;
        write(
            root.path(),
            &[".kb", "_intake", "decisions", "spaced.md"],
            "---\nweighin_item: \"wi-a b\"\nstatus: accepted\ntitle: Spaced\n---\n",
        )?;
        write(
            root.path(),
            &[".kb", "_intake", "decisions", "broken.md"],
            "---\nweighin_item: wi-000003\nstatus: accepted\n",
        )?;
        write(
            root.path(),
            &[".kb", "_intake", "decisions", "notes.txt"],
            "x",
        )?;

        assert_eq!(ids(root.path())?, ["wi-000002"]);
        Ok(())
    }

    #[test]
    fn rulings_come_first_then_atoms_each_in_id_order() -> TestResult {
        let root = tempfile::tempdir()?;
        ruling(root.path(), "zeta", "")?;
        ruling(root.path(), "alpha", "")?;
        atom(root.path(), "wi-bbbbbb", "accepted")?;
        atom(root.path(), "wi-aaaaaa", "accepted")?;

        assert_eq!(
            ids(root.path())?,
            ["alpha", "zeta", "wi-aaaaaa", "wi-bbbbbb"]
        );
        Ok(())
    }

    #[test]
    fn an_atom_a_ruling_already_cites_is_not_offered_twice() -> TestResult {
        let root = tempfile::tempdir()?;
        ruling(root.path(), "no-gui", "weigh_in_ref: wi-2415fc\n")?;
        atom(root.path(), "wi-2415fc", "accepted")?;

        assert_eq!(ids(root.path())?, ["no-gui"]);
        Ok(())
    }

    #[test]
    fn ids_that_cannot_be_options_and_the_reserved_none_are_skipped() -> TestResult {
        let root = tempfile::tempdir()?;
        ruling(root.path(), "none", "")?;
        ruling(root.path(), "a/b", "")?;
        ruling(root.path(), "kept", "")?;

        assert_eq!(ids(root.path())?, ["kept"]);
        Ok(())
    }

    #[test]
    fn at_most_31_precedents_are_offered() -> TestResult {
        let root = tempfile::tempdir()?;
        for n in 0..40 {
            ruling(root.path(), &format!("r{n:02}"), "")?;
        }

        let found = ids(root.path())?;

        assert_eq!(MAX_PRECEDENTS, 31, "32 options with none");
        assert_eq!(found.len(), MAX_PRECEDENTS);
        assert_eq!(found.first().map(String::as_str), Some("r00"));
        assert_eq!(found.last().map(String::as_str), Some("r30"));
        Ok(())
    }

    #[test]
    fn a_long_summary_is_cut_on_a_character_boundary() -> TestResult {
        let root = tempfile::tempdir()?;
        let long = "é".repeat(MAX_SUMMARY_BYTES);
        write(
            root.path(),
            &[".out-of-scope", "long.md"],
            &format!("---\nid: long\nruled_at: 2026-10-11\nsummary: {long}\n---\n"),
        )?;

        let found = gather(root.path())?;

        let summary = found.first().map(Precedent::summary).unwrap_or_default();
        assert!(summary.len() <= MAX_SUMMARY_BYTES, "{}", summary.len());
        assert!(
            summary.len() + 2 > MAX_SUMMARY_BYTES,
            "cut as late as it can be"
        );
        assert!(summary.chars().all(|c| c == 'é'), "{summary}");
        Ok(())
    }

    #[test]
    fn an_invalid_ruling_fails_naming_its_file() -> TestResult {
        let root = tempfile::tempdir()?;
        write(
            root.path(),
            &[".out-of-scope", "bad.md"],
            "no frontmatter\n",
        )?;

        let error = gather(root.path()).expect_err("an invalid ruling");

        assert!(matches!(error, PrecedentError::Rulings(_)), "{error:?}");
        Ok(())
    }

    #[test]
    fn a_ruling_store_over_its_file_bound_fails_rather_than_being_read_whole() -> TestResult {
        let root = tempfile::tempdir()?;
        for n in 0..=MAX_ATOM_FILES {
            ruling(root.path(), &format!("r{n:03}"), "")?;
        }

        let error = gather(root.path()).expect_err("over the bound");

        assert!(
            matches!(
                error,
                PrecedentError::Rulings(RulingError::TooMany {
                    limit: MAX_ATOM_FILES,
                    ..
                })
            ),
            "{error:?}"
        );
        Ok(())
    }

    #[test]
    fn a_repository_with_neither_store_has_no_precedents() -> TestResult {
        let root = tempfile::tempdir()?;

        assert!(gather(root.path())?.is_empty(), "nothing to offer");
        Ok(())
    }
}
