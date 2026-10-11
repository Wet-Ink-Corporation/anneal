//! git's global `-c` and `--config-env` settings that run something: an alias defines a git
//! command the guard cannot see, and keys such as `core.sshCommand` or `core.pager` run their
//! value as a shell command.

use super::GLOBAL_VALUES;
use crate::hook::guard::denylist::wrappers::{is_option, skip_one};
use crate::hook::guard::shell::Word;

/// Keys whose value git runs as a shell command (lower case).
const COMMAND_KEYS: [&str; 9] = [
    "core.sshcommand",
    "core.pager",
    "core.editor",
    "core.fsmonitor",
    "core.askpass",
    "sequence.editor",
    "diff.external",
    "credential.helper",
    "gpg.program",
];

/// What git's global settings do that the guard must see.
#[derive(Debug, Default)]
pub(in crate::hook::guard::denylist) struct Settings<'a> {
    /// A setting the guard cannot follow: an alias, a command whose value is in the
    /// environment, or a key known only at run time.
    pub(in crate::hook::guard::denylist) unseen: Option<String>,
    /// The values of settings git runs as shell commands.
    pub(in crate::hook::guard::denylist) scripts: Vec<&'a str>,
}

/// Reads the global settings of `git <args>`.
pub(in crate::hook::guard::denylist) fn settings(args: &[Word]) -> Settings<'_> {
    let mut settings = Settings::default();
    let mut rest = args;
    while let [first, tail @ ..] = rest {
        if !is_option(first) || settings.unseen.is_some() {
            break;
        }
        let text = first.text.as_str();
        let (value, after) = match text {
            "-c" | "--config-env" => (tail.first(), skip_one(tail)),
            _ if GLOBAL_VALUES.contains(&text) => (None, skip_one(tail)),
            _ => (None, tail),
        };
        match (text, value) {
            ("-c", Some(setting)) => settings.config(setting),
            ("--config-env", Some(setting)) => settings.config_env(&setting.text),
            _ => {
                if let Some(setting) = text.strip_prefix("--config-env=") {
                    settings.config_env(setting);
                }
            }
        }
        rest = after;
    }
    settings
}

impl<'a> Settings<'a> {
    /// `-c key=value`.
    fn config(&mut self, setting: &'a Word) {
        let (key, value) = setting
            .text
            .split_once('=')
            .unwrap_or((setting.text.as_str(), ""));
        let hidden = setting
            .expansions
            .first()
            .is_some_and(|expansion| expansion.range.start == 0);
        if hidden || is_alias(key) {
            self.unseen = Some(format!("-c {}", setting.text));
        } else if runs_command(key) {
            // A leading `!` runs the rest in a shell (aliases and credential helpers).
            self.scripts.push(value.strip_prefix('!').unwrap_or(value));
        }
    }

    /// `--config-env key=ENVVAR`: the value is in the environment.
    fn config_env(&mut self, setting: &str) {
        let key = setting.split_once('=').map_or(setting, |(key, _)| key);
        if is_alias(key) || runs_command(key) {
            self.unseen = Some(format!("--config-env {setting}"));
        }
    }
}

fn is_alias(key: &str) -> bool {
    key.get(.."alias.".len())
        .is_some_and(|section| section.eq_ignore_ascii_case("alias."))
}

/// Whether git runs the value of `key` as a command.
fn runs_command(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    let under = |section: &str, names: &[&str]| {
        key.starts_with(section) && names.iter().any(|name| key.ends_with(name))
    };
    COMMAND_KEYS.contains(&key.as_str())
        || key.starts_with("pager.")
        || under("filter.", &[".clean", ".smudge", ".process"])
        || under("diff.", &[".command", ".textconv"])
        || under("merge.", &[".driver"])
        || under("credential.", &[".helper"])
        || under("gpg.", &[".program"])
}
