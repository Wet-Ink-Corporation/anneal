//! The Tier 2 flags of `anneal eval routing` and the settings they resolve to.

use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Args, ValueEnum};

use super::claude::Options;
use crate::error::CliError;

/// The cost cap when `--max-cost-usd` is not given, in US dollars.
/// A full run of the repository's suite is capped at $25 (Ryan, wi-92bc1b); Tier 3 shares it.
pub(crate) const DEFAULT_MAX_COST_USD: f64 = 25.0;
/// The time limit when `--timeout` is not given.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3600);
/// Runs at once when `--concurrency` is not given.
pub(crate) const DEFAULT_CONCURRENCY: u8 = 4;

/// Which router `eval routing` measures.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum RouterKind {
    /// Tier 1: word overlap between each prompt and each skill's listing; 100% rank-1.
    #[default]
    Lexical,
    /// Tier 2: Claude Code's own router, through `claude plugin eval`; Wilson intervals.
    Claude,
}

/// The Tier 2 flags; each but `--router` needs `--router claude`.
#[derive(Debug, Clone, Default, Args)]
pub struct RealArgs {
    /// Which router to measure: `lexical` (Tier 1) or `claude` (Tier 2, runs a model; a run
    /// whose temporary workspace could not be removed still exits 0, with an
    /// `eval/routing/cleanup` finding)
    #[arg(long, value_enum, default_value_t = RouterKind::Lexical)]
    pub router: RouterKind,
    /// The `claude` program [default: `claude` on PATH]; on Windows an npm install is
    /// `claude.cmd`, so pass its full path
    #[arg(long, value_name = "PATH")]
    pub claude: Option<PathBuf>,
    /// The cost cap in US dollars: no batch starts once it is spent, though the runs already
    /// in flight can overshoot it [default: 25]
    #[arg(long, value_name = "USD", value_parser = cost_cap)]
    pub max_cost_usd: Option<f64>,
    /// The most the whole run may take, in seconds [default: 3600]
    #[arg(long, value_name = "SECS", value_parser = clap::value_parser!(u64).range(1..=86_400))]
    pub timeout: Option<u64>,
    /// Runs at once, 1 to 8 [default: 4]
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(1..=8))]
    pub concurrency: Option<u8>,
    /// The model every run uses [default: Claude Code's]
    #[arg(long, value_name = "MODEL")]
    pub model: Option<String>,
    /// Vouch for the plugin and its eval suite, so `claude plugin eval` does not stop to ask
    #[arg(long)]
    pub trust_plugin: bool,
    /// Run only when a skill's description, `when_to_use` or invocability, or the set of
    /// skills, changed since REF; otherwise report the run as skipped and exit 0
    #[arg(long, value_name = "REF")]
    pub changed_since: Option<String>,
}

/// A `--max-cost-usd` value: a finite amount above zero.
pub(crate) fn cost_cap(text: &str) -> Result<f64, String> {
    match text.parse::<f64>() {
        Ok(usd) if usd.is_finite() && usd > 0.0 => Ok(usd),
        Ok(_) | Err(_) => Err(format!(
            "`{text}` is not an amount of US dollars above zero"
        )),
    }
}

/// What `--router claude` runs with.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Settings<'a> {
    /// How `claude` is run.
    pub(crate) options: Options<'a>,
    /// The cost cap in US dollars.
    pub(crate) max_cost_usd: f64,
    /// The base to compare skills with, if the run is conditional.
    pub(crate) changed_since: Option<&'a str>,
}

impl RealArgs {
    /// The first Tier 2 flag given, by its name, when the router is not `claude`.
    fn stray_flag(&self) -> Option<&'static str> {
        [
            ("--claude", self.claude.is_some()),
            ("--max-cost-usd", self.max_cost_usd.is_some()),
            ("--timeout", self.timeout.is_some()),
            ("--concurrency", self.concurrency.is_some()),
            ("--model", self.model.is_some()),
            ("--trust-plugin", self.trust_plugin),
            ("--changed-since", self.changed_since.is_some()),
        ]
        .into_iter()
        .find_map(|(flag, given)| given.then_some(flag))
    }

    /// The Tier 2 settings, or `None` for the lexical router.
    ///
    /// # Errors
    /// [`CliError::Usage`] when a Tier 2 flag is given without `--router claude`, or
    /// `--changed-since` names something that is not a revision.
    pub(crate) fn settings(&self) -> Result<Option<Settings<'_>>, CliError> {
        if self.router == RouterKind::Lexical {
            return match self.stray_flag() {
                Some(flag) => Err(CliError::Usage {
                    reason: format!("{flag} needs --router claude"),
                }),
                None => Ok(None),
            };
        }
        if let Some(base) = self.changed_since.as_deref()
            && (base.is_empty() || base.starts_with('-'))
        {
            return Err(CliError::Usage {
                reason: format!("--changed-since {base:?} is not a revision"),
            });
        }
        Ok(Some(Settings {
            options: Options {
                program: self
                    .claude
                    .as_deref()
                    .unwrap_or_else(|| Path::new("claude")),
                model: self.model.as_deref(),
                concurrency: self.concurrency.unwrap_or(DEFAULT_CONCURRENCY),
                trust_plugin: self.trust_plugin,
                time_limit: self.timeout.map_or(DEFAULT_TIMEOUT, Duration::from_secs),
            },
            max_cost_usd: self.max_cost_usd.unwrap_or(DEFAULT_MAX_COST_USD),
            changed_since: self.changed_since.as_deref(),
        }))
    }
}

/// The plugin whose skills are in `skills`: its parent directory (`plugin/skills` is in
/// `plugin`), or the current directory for a bare `skills`.
pub(crate) fn plugin_of(skills: &Path) -> &Path {
    match skills.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        Some(_) | None => Path::new("."),
    }
}
