//! The flags of `anneal eval uplift` and the mode they resolve to.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::builder::{BoolishValueParser, TypedValueParser as _};
use clap::{ArgAction, Args};

use crate::error::CliError;
use crate::eval::real::args::{
    DEFAULT_CONCURRENCY, DEFAULT_MAX_COST_USD, DEFAULT_TIMEOUT, cost_cap,
};
use crate::eval::real::claude::Options;

/// Arguments of `anneal eval uplift`.
#[derive(Debug, Clone, Args)]
pub struct UpliftArgs {
    /// The directory of the plugin's skills; the plugin is its parent
    #[arg(long, value_name = "DIR", default_value = "plugin/skills")]
    pub skills: PathBuf,
    /// The directory of Tier 3 suites, `<skill>/{regression,capability}/<case>.json`
    #[arg(long, value_name = "DIR", default_value = "evals/uplift")]
    pub suites: PathBuf,
    /// Run only this skill's suites (repeatable)
    #[arg(long = "skill", value_name = "NAME")]
    pub skill: Vec<String>,
    /// Check the suites and report their coverage; run nothing
    #[arg(long)]
    pub check: bool,
    /// A plugin directory to run as the baseline instead of the plugin with each skill
    /// removed: an older version, or this plugin without a scaffolding hook
    #[arg(long, value_name = "DIR")]
    pub baseline: Option<PathBuf>,
    /// The `claude` program [default: `claude` on PATH]; on Windows an npm install is
    /// `claude.cmd`, so pass its full path
    #[arg(long, value_name = "PATH")]
    pub claude: Option<PathBuf>,
    /// The cost cap in US dollars: the with arm gets at most half of it, and no batch starts
    /// once it is spent, though the runs already in flight can overshoot it [default: 25]
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
    /// Run only the skills whose skill directory or suites changed since REF; with none
    /// changed, report the run as skipped and exit 0
    #[arg(long, value_name = "REF")]
    pub changed_since: Option<String>,
    /// Let cases grant the gated tools (Write, Edit, Bash) their files name; without it a run
    /// whose selected cases name one is refused before anything runs
    #[arg(
        long = "allow-gated-tools",
        action = ArgAction::SetTrue,
        value_parser = BoolishValueParser::new().map(GatedTools::from_flag)
    )]
    pub gated_tools: GatedTools,
}

/// Whether a run may grant the gated tools its cases name (`--allow-gated-tools`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatedTools {
    /// Refuse a run whose selected cases name a gated tool.
    Refuse,
    /// Grant them, as `--allow-gated-tools` asks.
    Allow,
}

impl GatedTools {
    /// [`Self::Allow`] when the flag was given.
    const fn from_flag(given: bool) -> Self {
        if given { Self::Allow } else { Self::Refuse }
    }
}

/// What `eval uplift` does.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Mode<'a> {
    /// Read and check the suites only.
    Check,
    /// Run the suites.
    Run(Settings<'a>),
}

/// What a run runs with.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Settings<'a> {
    /// How `claude` is run.
    pub(crate) options: Options<'a>,
    /// The cost cap in US dollars.
    pub(crate) max_cost_usd: f64,
    /// The base to compare with, if the run is conditional.
    pub(crate) changed_since: Option<&'a str>,
    /// The baseline plugin, if not each skill removed.
    pub(crate) baseline: Option<&'a Path>,
    /// The name of the plugin's skills directory (`--skills`' last component), from which a
    /// skill-removed copy removes the skill.
    pub(crate) skills_dir: &'a OsStr,
    /// Whether the selected cases may be granted the gated tools they name.
    pub(crate) gated_tools: GatedTools,
}

impl UpliftArgs {
    /// The first run flag given, by its name.
    fn run_flag(&self) -> Option<&'static str> {
        [
            ("--baseline", self.baseline.is_some()),
            ("--claude", self.claude.is_some()),
            ("--max-cost-usd", self.max_cost_usd.is_some()),
            ("--timeout", self.timeout.is_some()),
            ("--concurrency", self.concurrency.is_some()),
            ("--model", self.model.is_some()),
            ("--trust-plugin", self.trust_plugin),
            ("--changed-since", self.changed_since.is_some()),
            ("--allow-gated-tools", self.gated_tools == GatedTools::Allow),
        ]
        .into_iter()
        .find_map(|(flag, given)| given.then_some(flag))
    }

    /// The mode the flags ask for.
    ///
    /// # Errors
    /// [`CliError::Usage`] for a run flag with `--check`, a `--changed-since` that is not a
    /// revision, a `--baseline` that is not a directory, or, for a run, a `--skills` with no
    /// last component (ending in `..`).
    pub(crate) fn mode(&self) -> Result<Mode<'_>, CliError> {
        if self.check {
            return match self.run_flag() {
                Some(flag) => Err(CliError::Usage {
                    reason: format!("--check runs nothing, so {flag} does not apply"),
                }),
                None => Ok(Mode::Check),
            };
        }
        if let Some(base) = self.changed_since.as_deref()
            && (base.is_empty() || base.starts_with('-'))
        {
            return Err(CliError::Usage {
                reason: format!("--changed-since {base:?} is not a revision"),
            });
        }
        if let Some(dir) = self.baseline.as_deref()
            && !dir.is_dir()
        {
            return Err(CliError::Usage {
                reason: format!("--baseline {} is not a directory", dir.display()),
            });
        }
        let Some(skills_dir) = self.skills.file_name() else {
            return Err(CliError::Usage {
                reason: format!(
                    "--skills {} names no directory to remove a skill from",
                    self.skills.display()
                ),
            });
        };
        Ok(Mode::Run(Settings {
            skills_dir,
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
            baseline: self.baseline.as_deref(),
            gated_tools: self.gated_tools,
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{GatedTools, Mode, UpliftArgs};
    use crate::error::CliError;

    fn args() -> UpliftArgs {
        UpliftArgs {
            skills: PathBuf::from("plugin/skills"),
            suites: PathBuf::from("evals/uplift"),
            skill: Vec::new(),
            check: false,
            baseline: None,
            claude: None,
            max_cost_usd: None,
            timeout: None,
            concurrency: None,
            model: None,
            trust_plugin: false,
            changed_since: None,
            gated_tools: GatedTools::Refuse,
        }
    }

    fn reason(mode: Result<Mode<'_>, CliError>) -> Option<String> {
        match mode {
            Err(CliError::Usage { reason }) => Some(reason),
            Ok(_) | Err(_) => None,
        }
    }

    #[test]
    fn a_run_defaults_to_the_25_dollar_cap_and_the_skill_removed_baseline() {
        let args = args();

        let mode = args.mode();

        assert!(
            matches!(&mode, Ok(Mode::Run(settings))
                if settings.max_cost_usd.to_bits() == 25.0_f64.to_bits()
                    && settings.baseline.is_none()
                    && settings.changed_since.is_none()),
            "{mode:?}"
        );
    }

    #[test]
    fn a_baseline_that_is_not_a_directory_is_a_usage_error() {
        let args = UpliftArgs {
            baseline: Some(PathBuf::from("no/such/plugin")),
            ..args()
        };

        assert_eq!(
            reason(args.mode()),
            Some("--baseline no/such/plugin is not a directory".to_owned())
        );
    }

    #[test]
    fn a_run_removes_skills_from_the_directory_skills_names() {
        let args = UpliftArgs {
            skills: PathBuf::from("x/my-skills"),
            ..args()
        };

        let mode = args.mode();

        assert!(
            matches!(&mode, Ok(Mode::Run(settings)) if settings.skills_dir == "my-skills"),
            "{mode:?}"
        );
    }

    #[test]
    fn a_skills_path_with_no_directory_name_is_a_usage_error_for_a_run() {
        let args = UpliftArgs {
            skills: PathBuf::from("plugin/skills/.."),
            ..args()
        };

        assert_eq!(
            reason(args.mode()),
            Some("--skills plugin/skills/.. names no directory to remove a skill from".to_owned())
        );
    }

    #[test]
    fn a_changed_since_that_looks_like_an_option_is_a_usage_error() {
        let args = UpliftArgs {
            changed_since: Some("--all".to_owned()),
            ..args()
        };

        assert_eq!(
            reason(args.mode()),
            Some("--changed-since \"--all\" is not a revision".to_owned())
        );
    }

    #[test]
    fn check_names_the_first_run_flag_given() {
        let args = UpliftArgs {
            check: true,
            model: Some("m".to_owned()),
            trust_plugin: true,
            ..args()
        };

        assert_eq!(
            reason(args.mode()),
            Some("--check runs nothing, so --model does not apply".to_owned())
        );
    }

    #[test]
    fn a_run_refuses_gated_tools_unless_allow_gated_tools_is_given() {
        let refused = args();
        let allowed = UpliftArgs {
            gated_tools: GatedTools::Allow,
            ..args()
        };

        let modes = (refused.mode(), allowed.mode());

        assert!(
            matches!(
                &modes,
                (Ok(Mode::Run(refused)), Ok(Mode::Run(allowed)))
                    if refused.gated_tools == GatedTools::Refuse
                        && allowed.gated_tools == GatedTools::Allow
            ),
            "{modes:?}"
        );
    }

    #[test]
    fn allow_gated_tools_with_check_is_a_usage_error() {
        let args = UpliftArgs {
            check: true,
            gated_tools: GatedTools::Allow,
            ..args()
        };

        assert_eq!(
            reason(args.mode()),
            Some("--check runs nothing, so --allow-gated-tools does not apply".to_owned())
        );
    }
}
