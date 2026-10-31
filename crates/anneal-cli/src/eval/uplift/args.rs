//! The flags of `anneal eval uplift` and the mode they resolve to.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::builder::{BoolishValueParser, TypedValueParser as _};
use clap::{ArgAction, Args};

use crate::error::CliError;
use crate::eval::real::args::{
    DEFAULT_CONCURRENCY, DEFAULT_MAX_COST_USD, DEFAULT_TIMEOUT, cost_cap,
};
use crate::eval::real::claude::Options;
use crate::paths::{PathError, anneal_home_from};

use super::sandbox::denies_bash_writes;

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
    /// Let cases seed their runs' workspace from a fixture with a scaffold script, which runs
    /// outside the eval sandbox with your rights; without it a run whose selected cases name a
    /// fixture is refused before anything runs
    #[arg(
        long = "allow-scaffolds",
        action = ArgAction::SetTrue,
        value_parser = BoolishValueParser::new().map(Scaffolds::from_flag)
    )]
    pub scaffolds: Scaffolds,
    /// The directory the run's workspace is made in, made if missing (by default `$TMPDIR`
    /// when set, and on Linux not under the directories below, else `tmp` in the anneal home,
    /// else, with no home set, the system's temporary directory). `claude` runs with its
    /// `TMPDIR` inside the workspace, and on Linux the eval sandbox denies Bash every write
    /// under `/tmp`, `/var/tmp`, `/dev/shm` and `/run/shm`, so for a case that names Bash give
    /// a directory outside them. Keep it short, too: the sandbox's sockets go 88 bytes below
    /// it and a socket path holds at most 107, so on Linux a directory over 19 bytes (resolved)
    /// is warned about before the run, as Bash cannot start in any run
    #[arg(long, value_name = "DIR")]
    pub work_dir: Option<PathBuf>,
    /// Copy each batch's `claude plugin eval` result, which holds every run's grader
    /// verdicts, into DIR (made if missing) as `result-<batch>-<arm>.json`, where `<arm>` is
    /// `with`, `without-<skill>` or `baseline`, and each of its runs' traces as
    /// `traces/<batch>-<arm>/<case>-<run>.jsonl` (on Linux, where batches then pass
    /// `--keep-temp` so the traces outlive their runs; a trace outside the workspace is left
    /// where it is), so they outlive the workspace; a file of that name already there is
    /// overwritten, and one that cannot be written is reported as a finding without failing
    /// its batch
    #[arg(long, value_name = "DIR")]
    pub keep_results: Option<PathBuf>,
}

/// The variable whose value, when set and not empty, is the default work directory.
const TMPDIR_VAR: &str = "TMPDIR";

/// Where a run's workspace is made when `--work-dir` is not given: `$TMPDIR` when set, not
/// empty (the operator's choice of temporary directory) and, on Linux, not under a directory
/// the eval sandbox denies Bash writes under; else `<anneal home>/tmp`, which is outside them;
/// else, with no home set, the system's temporary directory. `lookup` reads one variable.
///
/// # Errors
/// [`PathError::RelativeHome`] when the default needs the anneal home and the variable that
/// names it holds a relative path.
pub(crate) fn default_work_dir(
    lookup: impl Fn(&str) -> Option<OsString>,
) -> Result<PathBuf, PathError> {
    if let Some(dir) = lookup(TMPDIR_VAR).filter(|dir| !dir.is_empty())
        && !denies_bash_writes(Path::new(&dir))
    {
        return Ok(PathBuf::from(dir));
    }
    match anneal_home_from(lookup) {
        Ok(home) => Ok(home.join("tmp")),
        // No home to put it in: this is only a default, and the system's temporary directory
        // still runs every case, only without Bash writes on Linux, which the run's
        // `eval/uplift/read-only-sandbox` finding then says.
        Err(PathError::NoHome) => Ok(std::env::temp_dir()),
        Err(error) => Err(error),
    }
}

/// Whether a run may run the scaffold scripts that seed its cases' fixtures
/// (`--allow-scaffolds`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scaffolds {
    /// Refuse a run whose selected cases name a fixture.
    Refuse,
    /// Run them, as `--allow-scaffolds` asks.
    Allow,
}

impl Scaffolds {
    /// [`Self::Allow`] when the flag was given.
    const fn from_flag(given: bool) -> Self {
        if given { Self::Allow } else { Self::Refuse }
    }
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
    /// Whether the selected cases' scaffold scripts may run.
    pub(crate) scaffolds: Scaffolds,
    /// `--work-dir`, the directory the run's workspace is made in; without it the run uses
    /// [`default_work_dir`].
    pub(crate) work_dir: Option<&'a Path>,
    /// `--keep-results`, where each batch's result is copied, if given.
    pub(crate) keep_results: Option<&'a Path>,
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
            ("--allow-scaffolds", self.scaffolds == Scaffolds::Allow),
            ("--work-dir", self.work_dir.is_some()),
            ("--keep-results", self.keep_results.is_some()),
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
            scaffolds: self.scaffolds,
            work_dir: self.work_dir.as_deref(),
            keep_results: self.keep_results.as_deref(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::{GatedTools, Mode, Scaffolds, UpliftArgs, default_work_dir};
    use crate::error::CliError;
    use crate::paths::PathError;

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
            scaffolds: Scaffolds::Refuse,
            work_dir: None,
            keep_results: None,
        }
    }

    /// A lookup that knows only `vars`.
    fn lookup(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let vars: Vec<(String, OsString)> = vars
            .iter()
            .map(|(name, value)| ((*name).to_owned(), OsString::from(value)))
            .collect();
        move |var| {
            vars.iter()
                .find(|(name, _)| name == var)
                .map(|(_, value)| value.to_owned())
        }
    }

    #[test]
    fn the_default_work_dir_is_tmpdir_when_it_is_set() {
        let dir = default_work_dir(lookup(&[("TMPDIR", "/scratch/me"), ("HOME", "/home/me")]))
            .expect("TMPDIR needs no home");

        assert_eq!(dir, PathBuf::from("/scratch/me"));
    }

    #[test]
    fn without_tmpdir_the_default_work_dir_is_the_anneal_home_s_tmp() {
        // A home must be absolute on the host, so both sit under its temp dir (`C:\..` on Windows).
        let me = std::env::temp_dir().join("me");
        let srv = std::env::temp_dir().join("srv-anneal");
        let unset =
            default_work_dir(lookup(&[("HOME", me.to_str().expect("UTF-8"))])).expect("a home");
        let empty = default_work_dir(lookup(&[
            ("TMPDIR", ""),
            ("ANNEAL_HOME", srv.to_str().expect("UTF-8")),
        ]))
        .expect("a home");

        assert_eq!(
            (unset, empty),
            (me.join(".anneal").join("tmp"), srv.join("tmp"))
        );
    }

    #[test]
    fn with_no_tmpdir_and_no_home_the_default_work_dir_is_the_system_temp_dir() {
        let dir = default_work_dir(lookup(&[])).expect("no home falls back");

        assert_eq!(dir, std::env::temp_dir());
    }

    #[test]
    fn on_linux_a_tmpdir_the_eval_sandbox_denies_gives_way_to_the_anneal_home_s_tmp() {
        let home = std::env::temp_dir().join("srv-anneal");
        let dir = default_work_dir(lookup(&[
            ("TMPDIR", "/var/tmp/me"),
            ("ANNEAL_HOME", home.to_str().expect("UTF-8")),
        ]))
        .expect("a home");

        let expected = if cfg!(target_os = "linux") {
            home.join("tmp")
        } else {
            PathBuf::from("/var/tmp/me")
        };
        assert_eq!(dir, expected);
    }

    #[test]
    fn a_relative_anneal_home_the_default_needs_is_an_error() {
        let error = default_work_dir(lookup(&[("ANNEAL_HOME", "rel/home")]));

        assert!(
            matches!(
                &error,
                Err(PathError::RelativeHome {
                    var: "ANNEAL_HOME",
                    ..
                })
            ),
            "{error:?}"
        );
    }

    #[test]
    fn a_run_keeps_the_work_dir_given() {
        let args = UpliftArgs {
            work_dir: Some(PathBuf::from("/var/eval")),
            ..args()
        };

        let mode = args.mode();

        assert!(
            matches!(&mode, Ok(Mode::Run(settings))
                if settings.work_dir == Some(std::path::Path::new("/var/eval"))),
            "{mode:?}"
        );
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
    fn a_run_refuses_scaffolds_unless_allow_scaffolds_is_given() {
        let refused = args();
        let allowed = UpliftArgs {
            scaffolds: Scaffolds::Allow,
            ..args()
        };

        let modes = (refused.mode(), allowed.mode());

        assert!(
            matches!(
                &modes,
                (Ok(Mode::Run(refused)), Ok(Mode::Run(allowed)))
                    if refused.scaffolds == Scaffolds::Refuse
                        && allowed.scaffolds == Scaffolds::Allow
            ),
            "{modes:?}"
        );
    }

    #[test]
    fn allow_scaffolds_with_check_is_a_usage_error() {
        let args = UpliftArgs {
            check: true,
            scaffolds: Scaffolds::Allow,
            ..args()
        };

        assert_eq!(
            reason(args.mode()),
            Some("--check runs nothing, so --allow-scaffolds does not apply".to_owned())
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
