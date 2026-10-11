//! Test support for the hooks that ask Clef: a fixed and a stepping clock, a configured anneal
//! home (and a way to block its log), and readers for the decision log and a hook's output. Declared `#[cfg(test)]`; its helpers
//! return `Result` so a failure names its cause (TST-14).

use std::collections::BTreeMap;
use std::error::Error;
use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTimeError};

use serde_json::{Value, json};
use tempfile::TempDir;

use crate::decide::clock::Clock;
use crate::decide::log::DecisionLog;
use crate::decide::point::Point;
use crate::decide::transport::TransportError;

/// A test's result: any error fails it with its `Debug` output.
pub(crate) type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// A clock stopped at 2025-10-09.
pub(crate) struct Fixed;

impl Clock for Fixed {
    fn unix_millis(&self) -> Result<u64, SystemTimeError> {
        Ok(1_760_000_000_000)
    }

    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }
}

/// A clock whose monotonic reading starts at zero and moves on by a step at each reading, so a
/// hook's budget runs out as fast as a test needs; its wall clock is [`Fixed`]'s.
pub(crate) struct Stepping {
    now_ms: AtomicU64,
    step_ms: u64,
}

impl Stepping {
    /// A clock moving on by `step_ms` milliseconds at each monotonic reading.
    pub(crate) const fn by_ms(step_ms: u64) -> Self {
        Self {
            now_ms: AtomicU64::new(0),
            step_ms,
        }
    }
}

impl Clock for Stepping {
    fn unix_millis(&self) -> Result<u64, SystemTimeError> {
        Fixed.unix_millis()
    }

    fn monotonic(&self) -> Duration {
        Duration::from_millis(self.now_ms.fetch_add(self.step_ms, Ordering::Relaxed))
    }
}

/// Puts a directory where the decision log's file goes, so no row can be appended while the
/// counts still read.
pub(crate) fn block_the_log(home: &Path) -> TestResult {
    std::fs::create_dir_all(home.join("decisions").join("decisions.jsonl"))?;
    Ok(())
}

/// An account and a token, so Clef is asked (in shadow, the default mode).
pub(crate) const CONFIGURED: [(&str, &str); 2] = [
    ("CLAUDE_PLUGIN_OPTION_CF_ACCOUNT_ID", "acct0123"),
    ("CLAUDE_PLUGIN_OPTION_CF_API_TOKEN", "s3cr3t-T0KEN"),
];

/// Workers AI's envelope around Clef's answer `inner`.
pub(crate) fn reply(inner: Result<Value, TransportError>) -> Result<Vec<u8>, TransportError> {
    inner.map(|inner| {
        json!({"result": {"response": inner.to_string()}})
            .to_string()
            .into_bytes()
    })
}

/// An anneal home of its own, the variables a hook sees, and a session transcript.
pub(crate) struct World {
    home: TempDir,
    vars: BTreeMap<String, OsString>,
}

impl World {
    /// A home with `vars` set (and `ANNEAL_HOME` pointing at it).
    pub(crate) fn new(vars: &[(&str, &str)]) -> TestResult<Self> {
        let home = tempfile::tempdir()?;
        let mut map: BTreeMap<String, OsString> = vars
            .iter()
            .map(|(name, value)| ((*name).to_owned(), OsString::from(value)))
            .collect();
        map.insert("ANNEAL_HOME".to_owned(), home.path().as_os_str().to_owned());
        Ok(Self { home, vars: map })
    }

    /// Configured, in shadow.
    pub(crate) fn configured() -> TestResult<Self> {
        Self::new(&CONFIGURED)
    }

    /// Configured for enforce, with `point` past its shadow period.
    pub(crate) fn enforcing(point: Point) -> TestResult<Self> {
        let mut vars = CONFIGURED.to_vec();
        vars.push(("CLAUDE_PLUGIN_OPTION_CLEF_MODE", "enforce"));
        let world = Self::new(&vars)?;
        let log = DecisionLog::in_home(world.home());
        for _ in 0..50 {
            log.count_answered(point)?;
        }
        Ok(world)
    }

    /// The anneal home.
    pub(crate) fn home(&self) -> &Path {
        self.home.path()
    }

    /// Reads one variable, as the hook's environment would.
    pub(crate) fn lookup(&self, name: &str) -> Option<OsString> {
        // The environment hands out its own copy of each value.
        self.vars.get(name).cloned()
    }

    /// The session's transcript.
    pub(crate) fn transcript(&self) -> PathBuf {
        self.home().join("session.jsonl")
    }

    /// `UserPromptSubmit` input for `prompt`, run in `cwd`.
    pub(crate) fn input(&self, prompt: &str, cwd: &Path) -> Vec<u8> {
        json!({
            "session_id": "s1",
            "transcript_path": self.transcript(),
            "cwd": cwd,
            "hook_event_name": "UserPromptSubmit",
            "prompt": prompt,
        })
        .to_string()
        .into_bytes()
    }

    /// Appends `lines` to the transcript.
    pub(crate) fn append_transcript(&self, lines: &[Value]) -> TestResult {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.transcript())?;
        for line in lines {
            writeln!(file, "{line}")?;
        }
        Ok(())
    }

    /// The decision log's rows, oldest first; none when there is no log.
    pub(crate) fn rows(&self) -> TestResult<Vec<Value>> {
        let path = self.home().join("decisions").join("decisions.jsonl");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let text = std::fs::read_to_string(path)?;
        let rows: Result<Vec<Value>, serde_json::Error> =
            text.lines().map(serde_json::from_str).collect();
        Ok(rows?)
    }
}

/// A hook's stdout parsed: `Null` when it wrote nothing, else its one JSON line.
pub(crate) fn parsed(out: &[u8]) -> TestResult<Value> {
    if out.is_empty() {
        return Ok(Value::Null);
    }
    let text = std::str::from_utf8(out)?;
    if text.lines().count() != 1 {
        return Err(format!("more than one line of output: {text}").into());
    }
    Ok(serde_json::from_str(text)?)
}

/// The output's `additionalContext`, if any.
pub(crate) fn context(output: &Value) -> Option<&str> {
    output
        .pointer("/hookSpecificOutput/additionalContext")
        .and_then(Value::as_str)
}
