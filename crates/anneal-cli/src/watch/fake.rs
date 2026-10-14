//! Test doubles for the watch's ports: a `gh` answering from recorded fixtures, a gate, and a
//! clock that only moves when the watch sleeps. No process, network or real time (TST-09,
//! TST-10).

use std::cell::{Cell, RefCell};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use crate::dag::tracker::github::{Field, GhApi, GhError, GhGraphql};
use crate::watch::gate::{GateBlock, HeadGate};
use crate::watch::poll::Clock;

/// The bytes of `tests/fixtures/watch/<name>`.
pub(crate) fn fixture(name: &str) -> io::Result<Vec<u8>> {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests/fixtures/watch", name]
        .iter()
        .collect();
    fs::read(path)
}

/// What the fake answers one call with.
#[derive(Debug, Clone, Copy)]
enum Answer {
    Fixture(&'static str),
    NotFound,
    Failed(&'static str),
}

/// The answers for calls whose endpoint contains `pattern` (`graphql` for GraphQL), given in
/// turn, the last for every later call.
#[derive(Debug)]
struct Route {
    pattern: &'static str,
    answers: RefCell<Vec<Answer>>,
}

/// A `gh` that answers each call by the longest route pattern its endpoint contains, and
/// remembers what it was asked.
#[derive(Debug, Default)]
pub(crate) struct FakeGh {
    routes: Vec<Route>,
    /// One line per call: the REST endpoint, or `graphql` and its fields as `-f name=value`.
    pub(crate) calls: RefCell<Vec<String>>,
}

impl FakeGh {
    /// A `gh` with no routes: every call fails.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Answers calls containing `pattern` with `fixtures` in turn.
    pub(crate) fn route(self, pattern: &'static str, fixtures: &[&'static str]) -> Self {
        let answers = fixtures.iter().map(|name| Answer::Fixture(name)).collect();
        self.with(pattern, answers)
    }

    /// Answers calls containing `pattern` with HTTP 404.
    pub(crate) fn missing(self, pattern: &'static str) -> Self {
        self.with(pattern, vec![Answer::NotFound])
    }

    /// Fails calls containing `pattern` (every call for `""`) with `stderr`.
    pub(crate) fn failing(self, pattern: &'static str, stderr: &'static str) -> Self {
        self.with(pattern, vec![Answer::Failed(stderr)])
    }

    fn with(mut self, pattern: &'static str, mut answers: Vec<Answer>) -> Self {
        answers.reverse();
        self.routes.push(Route {
            pattern,
            answers: RefCell::new(answers),
        });
        self
    }

    /// The answer to a call of `endpoint`.
    fn answer(&self, endpoint: &str) -> Result<Vec<u8>, GhError> {
        let failed = |stderr: &str| GhError::Failed {
            endpoint: endpoint.to_owned(),
            stderr: stderr.to_owned(),
        };
        let route = self
            .routes
            .iter()
            .filter(|route| endpoint.contains(route.pattern))
            .max_by_key(|route| route.pattern.len())
            .ok_or_else(|| failed("the fake gh has no route for this call"))?;
        let mut left = route.answers.borrow_mut();
        let next = if left.len() > 1 {
            left.pop()
        } else {
            left.last().copied()
        };
        match next.ok_or_else(|| failed("the fake gh route has no answer"))? {
            Answer::Fixture(name) => {
                fixture(name).map_err(|error| failed(&format!("fixture {name}: {error}")))
            }
            Answer::NotFound => Err(GhError::NotFound {
                endpoint: endpoint.to_owned(),
            }),
            Answer::Failed(stderr) => Err(failed(stderr)),
        }
    }
}

impl GhApi for FakeGh {
    fn get(&self, endpoint: &str) -> Result<Vec<u8>, GhError> {
        self.calls.borrow_mut().push(endpoint.to_owned());
        self.answer(endpoint)
    }
}

impl GhGraphql for FakeGh {
    fn graphql(&self, _query: &str, fields: &[Field<'_>]) -> Result<Vec<u8>, GhError> {
        let shown: Vec<String> = fields
            .iter()
            .map(|field| match field {
                Field::Raw(name, value) => format!("-f {name}={value}"),
                Field::Typed(name, value) => format!("-F {name}={value}"),
            })
            .collect();
        self.calls
            .borrow_mut()
            .push(format!("graphql {}", shown.join(" ")));
        self.answer("graphql")
    }
}

/// A `gh` for pull request 7: the REST pull request, its head's check runs and combined
/// status, and its review threads by GraphQL.
pub(crate) fn pull_request(
    pull: &'static str,
    checks: &'static str,
    status: &'static str,
    threads: &'static str,
) -> FakeGh {
    FakeGh::new()
        .route("/pulls/7", &[pull])
        .route("/check-runs", &[checks])
        .route("/status", &[status])
        .route("graphql", &[threads])
}

/// A merge gate that allows, or denies every head with one reason, and remembers what it
/// judged.
#[derive(Debug, Default)]
pub(crate) struct FakeGate {
    denial: Option<&'static str>,
    /// The pull requests and heads judged.
    pub(crate) judged: RefCell<Vec<(u64, String)>>,
}

impl FakeGate {
    /// A gate that denies with check `no-verdict` and `reason`.
    pub(crate) fn denying(reason: &'static str) -> Self {
        Self {
            denial: Some(reason),
            judged: RefCell::new(Vec::new()),
        }
    }
}

impl HeadGate for FakeGate {
    fn judge(&self, pr: u64, head: &str) -> Option<GateBlock> {
        self.judged.borrow_mut().push((pr, head.to_owned()));
        self.denial.map(|reason| GateBlock {
            check: "no-verdict",
            reason: reason.to_owned(),
        })
    }
}

/// A clock that only moves when the watch sleeps, and remembers every sleep.
#[derive(Debug, Default)]
pub(crate) struct FakeClock {
    now: Cell<Duration>,
    /// Every sleep, in order.
    pub(crate) sleeps: RefCell<Vec<Duration>>,
}

impl Clock for FakeClock {
    fn elapsed(&self) -> Duration {
        self.now.get()
    }

    fn sleep(&self, delay: Duration) {
        self.sleeps.borrow_mut().push(delay);
        self.now.set(self.now.get().saturating_add(delay));
    }
}
