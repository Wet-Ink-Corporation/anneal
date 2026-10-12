//! Finding the `weighin` CLI: `$WEIGHIN_BIN`, then `weighin` on `PATH`.
//!
//! weigh-in ships its binary inside its own plugin (`${CLAUDE_PLUGIN_ROOT}/libexec/weighin`, a
//! root anneal cannot see), so the human or the plugin points `$WEIGHIN_BIN` at it; otherwise an
//! installed `weighin` on `PATH` is used. On Windows the names tried are `weighin.exe`,
//! `weighin.cmd` and `weighin.bat`, in that order.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::BridgeError;

/// The variable naming the CLI.
pub(crate) const BIN_VAR: &str = "WEIGHIN_BIN";

/// The file names looked for in each `PATH` directory.
#[cfg(windows)]
const NAMES: [&str; 3] = ["weighin.exe", "weighin.cmd", "weighin.bat"];
/// The file names looked for in each `PATH` directory.
#[cfg(not(windows))]
const NAMES: [&str; 1] = ["weighin"];

/// Where the CLI was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    /// `$WEIGHIN_BIN`.
    Env,
    /// A directory on `PATH`.
    Path,
}

/// The CLI to run, and where it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Located {
    /// The program.
    pub(crate) path: PathBuf,
    /// How it was found.
    pub(crate) source: Source,
}

/// The `weighin` CLI, from the variables `lookup` reads: a non-empty `$WEIGHIN_BIN` must name a
/// file; otherwise the first `weighin` file on `PATH`.
///
/// # Errors
/// [`BridgeError::NotAFile`] when `$WEIGHIN_BIN` names no file, [`BridgeError::NotFound`] when it
/// is unset or empty and no `PATH` directory holds one.
pub(crate) fn locate(lookup: impl Fn(&str) -> Option<OsString>) -> Result<Located, BridgeError> {
    if let Some(bin) = lookup(BIN_VAR).filter(|value| !value.is_empty()) {
        let path = PathBuf::from(bin);
        return if is_file(&path) {
            Ok(Located {
                path,
                source: Source::Env,
            })
        } else {
            Err(BridgeError::NotAFile { path })
        };
    }
    let dirs = lookup("PATH").unwrap_or_default();
    std::env::split_paths(&dirs)
        .filter(|dir| !dir.as_os_str().is_empty())
        .flat_map(|dir| NAMES.map(|name| dir.join(name)))
        .find(|candidate| is_file(candidate))
        .map(|path| Located {
            path,
            source: Source::Path,
        })
        .ok_or(BridgeError::NotFound)
}

/// Whether `path` is a file, following symlinks (a launcher is often linked).
fn is_file(path: &Path) -> bool {
    path.metadata().is_ok_and(|meta| meta.is_file())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::fs;
    use std::path::Path;

    use super::{Located, Source, locate};
    use crate::weighin::BridgeError;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// The file name `PATH` is searched for on this platform.
    const PROGRAM: &str = if cfg!(windows) {
        "weighin.exe"
    } else {
        "weighin"
    };

    fn lookup(vars: &[(&str, &OsString)]) -> impl Fn(&str) -> Option<OsString> {
        let map: BTreeMap<String, OsString> = vars
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        // The environment hands out its own copy of each value.
        move |name| map.get(name).cloned()
    }

    fn path_of(dirs: &[&Path]) -> Result<OsString, std::env::JoinPathsError> {
        std::env::join_paths(dirs)
    }

    #[test]
    fn weighin_bin_wins_over_path() -> TestResult {
        let dir = tempfile::tempdir()?;
        let named = dir.path().join("pinned-weighin");
        fs::write(&named, "")?;
        fs::write(dir.path().join(PROGRAM), "")?;
        let path = path_of(&[dir.path()])?;

        let found = locate(lookup(&[
            ("WEIGHIN_BIN", &named.clone().into_os_string()),
            ("PATH", &path),
        ]))?;

        assert_eq!(
            found,
            Located {
                path: named,
                source: Source::Env
            }
        );
        Ok(())
    }

    #[test]
    fn an_empty_weighin_bin_falls_back_to_the_first_path_directory_holding_one() -> TestResult {
        let empty = tempfile::tempdir()?;
        let holding = tempfile::tempdir()?;
        fs::write(holding.path().join(PROGRAM), "")?;
        let path = path_of(&[empty.path(), holding.path()])?;

        let found = locate(lookup(&[
            ("WEIGHIN_BIN", &OsString::new()),
            ("PATH", &path),
        ]))?;

        assert_eq!(
            found,
            Located {
                path: holding.path().join(PROGRAM),
                source: Source::Path
            }
        );
        Ok(())
    }

    #[test]
    fn a_weighin_bin_naming_no_file_is_not_a_file_even_with_one_on_path() -> TestResult {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join(PROGRAM), "")?;
        let missing = dir.path().join("missing");
        let path = path_of(&[dir.path()])?;

        let error = locate(lookup(&[
            ("WEIGHIN_BIN", &missing.clone().into_os_string()),
            ("PATH", &path),
        ]))
        .expect_err("named but missing");

        assert!(
            matches!(&error, BridgeError::NotAFile { path } if *path == missing),
            "{error:?}"
        );
        let directory = locate(lookup(&[(
            "WEIGHIN_BIN",
            &dir.path().as_os_str().to_owned(),
        )]));
        assert!(
            matches!(directory, Err(BridgeError::NotAFile { .. })),
            "a directory is no program: {directory:?}"
        );
        Ok(())
    }

    #[test]
    fn with_neither_it_is_not_found() -> TestResult {
        let dir = tempfile::tempdir()?;
        fs::create_dir(dir.path().join(PROGRAM))?;
        let path = path_of(&[dir.path()])?;

        let error = locate(lookup(&[("PATH", &path)])).expect_err("a directory named weighin");
        let unset = locate(lookup(&[])).expect_err("no PATH at all");

        assert!(matches!(error, BridgeError::NotFound), "{error:?}");
        assert!(matches!(unset, BridgeError::NotFound), "{unset:?}");
        Ok(())
    }
}
