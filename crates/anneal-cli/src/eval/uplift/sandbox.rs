//! Whether `claude plugin eval`'s sandbox lets Bash write in a run whose workspace is under a
//! directory.
//!
//! On Linux `claude plugin eval` writes each run's sandbox settings with its own `home/` and
//! `tmp/` in `allowWrite`, and `/tmp`, `/var/tmp`, `/dev/shm` and `/run/shm` in `denyWrite`;
//! the deny wins, so a run made under one of those can write nothing from Bash, the anneal
//! ledger included (read from Claude Code 2.1.314's bundle). macOS makes its runs under `/tmp`
//! and leaves that out of the deny list, so only Linux is affected.

use std::fs;
use std::path::Path;

/// The directories the Linux eval sandbox denies Bash every write under.
const DENIED_ROOTS: [&str; 4] = ["/tmp", "/var/tmp", "/dev/shm", "/run/shm"];

/// The longest path a Unix socket binds on Linux: `sun_path` holds 108 bytes, the NUL included.
pub(crate) const SUN_PATH_MAX: usize = 107;
/// What `claude plugin eval` puts below the `TMPDIR` it is given for each run's own `TMPDIR`
/// on Linux: `realpath(mkdtemp(<TMPDIR>/claude-eval-))` then `tmp` (Claude Code 2.1.314).
const RUN_TMP: &str = "/claude-eval-XXXXXX/tmp";
/// The longer of the two bridge sockets the run's Bash sandbox binds in its `TMPDIR`
/// (`claude-http-<16 hex>.sock` is one byte shorter).
const BRIDGE_SOCKET: &str = "/claude-socks-0123456789abcdef.sock";
/// The longest `TMPDIR` the `claude` child can have on Linux for its runs' bridge sockets to
/// bind: 107 - 23 - 35 = 49 bytes.
pub(crate) const CHILD_TMPDIR_BUDGET: usize = SUN_PATH_MAX - RUN_TMP.len() - BRIDGE_SOCKET.len();

/// By how many bytes a child `TMPDIR` of `len` bytes is over [`CHILD_TMPDIR_BUDGET`], if it is.
pub(crate) fn overrun(len: usize) -> Option<usize> {
    len.checked_sub(CHILD_TMPDIR_BUDGET)
        .filter(|over| *over > 0)
}

/// By how many bytes a `claude` child whose `TMPDIR`, resolved as `claude` resolves it, is
/// `len` bytes long is over [`CHILD_TMPDIR_BUDGET`] on this host: only on Linux, as macOS
/// makes its runs under `/tmp/e-XXXXXX` whatever the `TMPDIR`.
pub(crate) fn socket_overrun(len: usize) -> Option<usize> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    overrun(len)
}

/// Whether `dir` is one of [`DENIED_ROOTS`] or inside one, compared component by component
/// (`/tmpfiles` is not inside `/tmp`).
fn under_denied_root(dir: &Path) -> bool {
    DENIED_ROOTS.iter().any(|root| dir.starts_with(root))
}

/// Whether Bash in a run whose workspace is made under `work_dir` can write nothing on this
/// host: on Linux, `work_dir` resolved (links followed) is under a denied directory.
pub(crate) fn denies_bash_writes(work_dir: &Path) -> bool {
    if !cfg!(target_os = "linux") {
        return false;
    }
    // The sandbox judges resolved paths; a directory that cannot be resolved is judged as
    // written, which is the best this check can do without it.
    let resolved = fs::canonicalize(work_dir);
    under_denied_root(resolved.as_deref().unwrap_or(work_dir))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{CHILD_TMPDIR_BUDGET, overrun, under_denied_root};

    #[test]
    fn the_linux_child_tmpdir_budget_is_49_bytes() {
        assert_eq!(CHILD_TMPDIR_BUDGET, 49);
    }

    #[test]
    fn a_child_tmpdir_over_the_budget_overruns_by_its_excess() {
        let overruns = [0, 49, 50, 57].map(overrun);

        assert_eq!(overruns, [None, None, Some(1), Some(8)]);
    }

    #[test]
    fn a_directory_in_or_at_a_denied_root_is_denied() {
        let denied = [
            "/tmp",
            "/tmp/anneal",
            "/var/tmp/x",
            "/dev/shm/x",
            "/run/shm",
        ]
        .map(|dir| under_denied_root(Path::new(dir)));

        assert_eq!(denied, [true; 5]);
    }

    #[test]
    fn a_directory_beside_or_above_a_denied_root_is_not() {
        let denied = ["/tmpfiles", "/home/me/.anneal/tmp", "/var", "/", "tmp/x"]
            .map(|dir| under_denied_root(Path::new(dir)));

        assert_eq!(denied, [false; 5]);
    }
}
