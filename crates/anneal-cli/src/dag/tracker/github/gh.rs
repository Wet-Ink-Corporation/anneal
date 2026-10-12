//! The `gh` ports ([`GhApi`] for REST, [`GhGraphql`] for read-only GraphQL queries) and their
//! real adapter ([`GhCli`]), shared by `dag`'s GitHub tracker and `anneal watch`.

use std::ffi::OsString;
use std::io;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::process::{self, WaitError};

/// How long one `gh api` call may take before it is killed (H-13).
pub const GH_TIMEOUT: Duration = Duration::from_secs(30);
/// The most bytes kept from one `gh api` call's output (H-13).
pub const MAX_GH_OUTPUT: u64 = 16 * 1024 * 1024;

/// What `gh` prints on stderr when the API answers 404 Not Found or 410 Gone (a deleted
/// issue): either way the issue is missing.
const MISSING: [&str; 2] = ["(HTTP 404)", "(HTTP 410)"];

/// Read-only access to GitHub's REST API: the port the reader depends on.
pub trait GhApi {
    /// The response body of `GET <endpoint>` (relative to the API root, query included).
    ///
    /// # Errors
    /// [`GhError::NotFound`] when GitHub answers 404 or 410; another [`GhError`] when the call fails
    /// or its answer is unusable.
    fn get(&self, endpoint: &str) -> Result<Vec<u8>, GhError>;
}

/// Why a `gh api` call failed.
#[derive(Debug, thiserror::Error)]
pub enum GhError {
    /// `gh` could not be started (not installed, not on PATH).
    #[error("running gh")]
    Spawn(#[source] io::Error),
    /// `gh` could not be waited on or its output read.
    #[error("waiting for gh api {endpoint}")]
    Wait {
        /// The endpoint asked for.
        endpoint: String,
        /// The I/O failure.
        #[source]
        source: io::Error,
    },
    /// `gh` did not answer in time and was killed.
    #[error("gh api {endpoint} did not answer within {}s", timeout.as_secs())]
    Timeout {
        /// The endpoint asked for.
        endpoint: String,
        /// The limit.
        timeout: Duration,
    },
    /// GitHub answered 404 or 410: no such issue, a deleted one, or one we cannot see.
    #[error("gh api {endpoint}: not found")]
    NotFound {
        /// The endpoint asked for.
        endpoint: String,
    },
    /// `gh` exited non-zero for another reason (not logged in, rate limited, offline).
    #[error("gh api {endpoint} failed: {stderr}")]
    Failed {
        /// The endpoint asked for.
        endpoint: String,
        /// What `gh` said, trimmed.
        stderr: String,
    },
    /// The answer reached [`MAX_GH_OUTPUT`], so it may be cut short.
    #[error("gh api {endpoint} printed {MAX_GH_OUTPUT} bytes or more")]
    TooLarge {
        /// The endpoint asked for.
        endpoint: String,
    },
}

/// The real `gh`, run as `gh api --method GET <endpoint>` with prompts and colour off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhCli {
    /// The program run; `gh` outside tests.
    pub(super) program: OsString,
    /// The limit per call.
    pub(super) timeout: Duration,
}

impl GhCli {
    /// `gh` from PATH, killed after [`GH_TIMEOUT`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            program: OsString::from("gh"),
            timeout: GH_TIMEOUT,
        }
    }
}

impl Default for GhCli {
    fn default() -> Self {
        Self::new()
    }
}

impl GhApi for GhCli {
    fn get(&self, endpoint: &str) -> Result<Vec<u8>, GhError> {
        self.run(endpoint, ["api", "--method", "GET", endpoint])
    }
}

/// One variable of a GraphQL query, as `gh api graphql` takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field<'a> {
    /// `-f name=value`: sent as a string as given, never read as a number, a placeholder or a
    /// file. Anything that comes from the command line goes this way.
    Raw(&'a str, &'a str),
    /// `-F name=value`: gh turns integers and `true`/`false`/`null` into JSON values, fills in
    /// `{owner}`, `{repo}` and `{branch}` from the current directory's repository, and reads a
    /// value starting with `@` from a file, so only values anneal builds itself go this way.
    Typed(&'a str, &'a str),
}

/// Read-only GraphQL queries: the port `anneal watch` reads pull requests and checks through.
pub trait GhGraphql {
    /// The response body of the GraphQL `query` with `fields` as its variables. A query reads;
    /// a mutation is never sent.
    ///
    /// # Errors
    /// A [`GhError`] when the call fails (GitHub reporting an error in the query is a failure
    /// of the call) or its answer is unusable.
    fn graphql(&self, query: &str, fields: &[Field<'_>]) -> Result<Vec<u8>, GhError>;
}

impl GhGraphql for GhCli {
    fn graphql(&self, query: &str, fields: &[Field<'_>]) -> Result<Vec<u8>, GhError> {
        self.run("graphql", graphql_args(query, fields))
    }
}

/// The arguments of `gh api graphql` for `query` and `fields`.
pub(super) fn graphql_args(query: &str, fields: &[Field<'_>]) -> Vec<OsString> {
    let mut args = Vec::with_capacity(fields.len().saturating_add(2).saturating_mul(2));
    args.extend(["api", "graphql", "-f"].map(OsString::from));
    args.push(OsString::from(format!("query={query}")));
    for field in fields {
        let (flag, name, value) = match field {
            Field::Raw(name, value) => ("-f", name, value),
            Field::Typed(name, value) => ("-F", name, value),
        };
        args.push(OsString::from(flag));
        args.push(OsString::from(format!("{name}={value}")));
    }
    args
}

impl GhCli {
    /// Runs `gh <args>` with prompts and colour off, within the timeout and output cap; the
    /// answer of a call that exits 0. `endpoint` names the call in errors.
    fn run<I, S>(&self, endpoint: &str, args: I) -> Result<Vec<u8>, GhError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let mut child = Command::new(&self.program)
            .args(args)
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env("NO_COLOR", "1")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(GhError::Spawn)?;
        let output = process::wait_with_deadline(&mut child, self.timeout, MAX_GH_OUTPUT, "gh")
            .map_err(|error| match error {
                WaitError::Io(source) => GhError::Wait {
                    endpoint: endpoint.to_owned(),
                    source,
                },
                WaitError::Timeout => GhError::Timeout {
                    endpoint: endpoint.to_owned(),
                    timeout: self.timeout,
                },
            })?;
        if !output.status.success() {
            return Err(failure(endpoint, &output.stderr));
        }
        within_cap(endpoint, output.stdout.len())?;
        Ok(output.stdout)
    }
}

/// Refuses an answer of `len` bytes that reached [`MAX_GH_OUTPUT`]: the capture keeps at most
/// that many, so a full capture may be cut short.
pub(super) fn within_cap(endpoint: &str, len: usize) -> Result<(), GhError> {
    if u64::try_from(len).map_or(true, |len| len >= MAX_GH_OUTPUT) {
        return Err(GhError::TooLarge {
            endpoint: endpoint.to_owned(),
        });
    }
    Ok(())
}

/// The error for a `gh api` call that exited non-zero, from its stderr: [`GhError::NotFound`]
/// when `gh` reports HTTP 404 or 410, [`GhError::Failed`] otherwise.
pub(super) fn failure(endpoint: &str, stderr: &[u8]) -> GhError {
    let endpoint = endpoint.to_owned();
    let stderr = String::from_utf8_lossy(stderr).trim().to_owned();
    if MISSING.iter().any(|status| stderr.contains(status)) {
        GhError::NotFound { endpoint }
    } else {
        GhError::Failed { endpoint, stderr }
    }
}
