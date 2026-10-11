//! The plugin's `userConfig` as hooks see it: the account, the token and the Clef mode.
//!
//! Claude Code (checked in 2.1.296) gives a hook each `userConfig` value, sensitive ones
//! included, as `CLAUDE_PLUGIN_OPTION_<KEY>`, the key upper-cased with every character outside
//! `[A-Za-z0-9_]` turned into `_`. An empty account means Clef is off. The mode is
//! `<global>[,<point>=<mode>]...`, each mode `off`, `shadow` or `enforce`, spaces around the
//! parts ignored; empty or unset is `shadow`, the manifest's default.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;

use serde::Serialize;

use crate::decide::point::Point;
use crate::decide::transport::ApiToken;

/// `userConfig.cf_account_id`: the Cloudflare account that runs Clef.
pub const ACCOUNT_VAR: &str = "CLAUDE_PLUGIN_OPTION_CF_ACCOUNT_ID";
/// `userConfig.cf_api_token` (sensitive): the Workers AI token.
pub const TOKEN_VAR: &str = "CLAUDE_PLUGIN_OPTION_CF_API_TOKEN";
/// `userConfig.clef_mode`: `off`, `shadow` or `enforce`, with per-point overrides.
pub const MODE_VAR: &str = "CLAUDE_PLUGIN_OPTION_CLEF_MODE";

/// Longest account id.
const MAX_ACCOUNT_BYTES: usize = 64;

/// How a decision point's answer is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Clef is not asked.
    Off,
    /// Clef is asked and the would-be action logged; callers do not act on it.
    Shadow,
    /// Clef is asked and callers act on the action.
    Enforce,
}

impl Mode {
    /// The name as configured.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Shadow => "shadow",
            Self::Enforce => "enforce",
        }
    }

    /// The mode named `name`, if any.
    fn named(name: &str) -> Option<Self> {
        [Self::Off, Self::Shadow, Self::Enforce]
            .into_iter()
            .find(|mode| mode.as_str() == name)
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why the configuration could not be read. No variant carries the token.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// A variable is not valid UTF-8.
    #[error("{var} is not valid UTF-8")]
    NotUtf8 {
        /// The variable.
        var: &'static str,
    },
    /// The account id is not 1 to 64 ASCII letters and digits.
    #[error("{ACCOUNT_VAR} must be 1 to 64 ASCII letters and digits")]
    Account,
    /// The token is not printable ASCII without whitespace. Its value is never shown.
    #[error("{TOKEN_VAR} is not a valid token (printable ASCII, no whitespace)")]
    Token,
    /// The mode does not parse.
    #[error(
        "{MODE_VAR} {value:?} is not `<off|shadow|enforce>[,<D1..D7>=<off|shadow|enforce>]...`"
    )]
    Mode {
        /// The value as set.
        value: String,
    },
}

/// The account, token and mode a decision runs with.
#[derive(Debug)]
pub struct ClefConfig {
    account: Option<String>,
    token: Option<ApiToken>,
    global: Mode,
    overrides: BTreeMap<Point, Mode>,
}

impl ClefConfig {
    /// Reads the configuration through `lookup`; empty values count as unset.
    ///
    /// # Errors
    /// [`ConfigError`] for a value that is not UTF-8, an account that is not alphanumeric, a
    /// token unfit for a header, or a mode that does not parse.
    pub fn from_env(lookup: impl Fn(&str) -> Option<OsString>) -> Result<Self, ConfigError> {
        let read = |var: &'static str| -> Result<Option<String>, ConfigError> {
            match lookup(var) {
                None => Ok(None),
                Some(value) => value
                    .into_string()
                    .map(|text| Some(text).filter(|text| !text.is_empty()))
                    .map_err(|_not_utf8| ConfigError::NotUtf8 { var }),
            }
        };
        let account = read(ACCOUNT_VAR)?;
        if let Some(account) = &account {
            let valid = account.len() <= MAX_ACCOUNT_BYTES
                && account.bytes().all(|byte| byte.is_ascii_alphanumeric());
            if !valid {
                return Err(ConfigError::Account);
            }
        }
        let token = read(TOKEN_VAR)?
            .map(|value| ApiToken::new(value).ok_or(ConfigError::Token))
            .transpose()?;
        let (global, overrides) = match read(MODE_VAR)? {
            None => (Mode::Shadow, BTreeMap::new()),
            Some(value) => parse_mode(&value).ok_or(ConfigError::Mode { value })?,
        };
        Ok(Self {
            account,
            token,
            global,
            overrides,
        })
    }

    /// The account id, or `None` when Clef is off for want of one.
    #[must_use]
    pub fn account(&self) -> Option<&str> {
        self.account.as_deref()
    }

    /// The token, if one is set.
    #[must_use]
    pub const fn token(&self) -> Option<&ApiToken> {
        self.token.as_ref()
    }

    /// The configured mode of `point`: its override, else the global mode.
    #[must_use]
    pub fn mode(&self, point: Point) -> Mode {
        self.overrides.get(&point).copied().unwrap_or(self.global)
    }
}

/// `<global>[,<point>=<mode>]...`, or `None` when it does not parse.
fn parse_mode(value: &str) -> Option<(Mode, BTreeMap<Point, Mode>)> {
    let mut parts = value.split(',').map(str::trim);
    let global = Mode::named(parts.next()?)?;
    let mut overrides = BTreeMap::new();
    for part in parts {
        let (point, mode) = part.split_once('=')?;
        let point: Point = point.trim().parse().ok()?;
        overrides.insert(point, Mode::named(mode.trim())?);
    }
    Some((global, overrides))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsString;

    use super::{ACCOUNT_VAR, ClefConfig, ConfigError, MODE_VAR, Mode, TOKEN_VAR};
    use crate::decide::point::Point;
    use crate::decide::transport::ApiToken;

    const SECRET: &str = "s3cr3t-T0KEN-value";

    fn config(vars: &[(&str, &str)]) -> Result<ClefConfig, ConfigError> {
        let vars: BTreeMap<String, OsString> = vars
            .iter()
            .map(|(name, value)| ((*name).to_owned(), OsString::from(value)))
            .collect();
        ClefConfig::from_env(|name| vars.get(name).cloned())
    }

    #[test]
    fn the_variables_are_claude_codes_upper_cased_user_config_keys() {
        assert_eq!(
            [ACCOUNT_VAR, TOKEN_VAR, MODE_VAR],
            [
                "CLAUDE_PLUGIN_OPTION_CF_ACCOUNT_ID",
                "CLAUDE_PLUGIN_OPTION_CF_API_TOKEN",
                "CLAUDE_PLUGIN_OPTION_CLEF_MODE",
            ]
        );
    }

    #[test]
    fn account_token_and_mode_come_from_the_environment() -> Result<(), ConfigError> {
        let config = config(&[
            (ACCOUNT_VAR, "0123abcd"),
            (TOKEN_VAR, SECRET),
            (MODE_VAR, "enforce"),
        ])?;

        assert_eq!(config.account(), Some("0123abcd"));
        assert_eq!(config.token().map(ApiToken::expose_secret), Some(SECRET));
        assert_eq!(config.mode(Point::D3), Mode::Enforce);
        Ok(())
    }

    #[test]
    fn an_empty_or_missing_account_means_clef_is_off() -> Result<(), ConfigError> {
        assert_eq!(
            config(&[(ACCOUNT_VAR, ""), (TOKEN_VAR, SECRET)])?.account(),
            None
        );
        assert_eq!(config(&[])?.account(), None);
        Ok(())
    }

    #[test]
    fn the_mode_defaults_to_shadow() -> Result<(), ConfigError> {
        for vars in [&[][..], &[(MODE_VAR, "")]] {
            let config = config(vars)?;
            assert_eq!(config.mode(Point::D1), Mode::Shadow, "{vars:?}");
        }
        Ok(())
    }

    #[test]
    fn a_per_point_override_wins_over_the_global_mode() -> Result<(), ConfigError> {
        let config = config(&[(MODE_VAR, "shadow, D1=off ,D3=enforce")])?;

        assert_eq!(config.mode(Point::D1), Mode::Off);
        assert_eq!(config.mode(Point::D2), Mode::Shadow);
        assert_eq!(config.mode(Point::D3), Mode::Enforce);
        Ok(())
    }

    #[test]
    fn an_invalid_mode_is_refused_with_the_value() {
        for value in [
            "loud",
            "shadow,D9=off",
            "shadow,D1",
            "D1=off",
            "shadow,D1=maybe",
        ] {
            assert!(
                matches!(config(&[(MODE_VAR, value)]), Err(ConfigError::Mode { .. })),
                "{value:?} is refused"
            );
        }
    }

    #[test]
    fn an_account_that_is_not_alphanumeric_is_refused() {
        for account in ["abc/def", "a b", "x?y"] {
            assert!(
                matches!(config(&[(ACCOUNT_VAR, account)]), Err(ConfigError::Account)),
                "{account:?} is refused"
            );
        }
    }

    #[test]
    fn a_bad_token_is_refused_without_its_value() {
        let leaked = "has space";

        let error = config(&[(TOKEN_VAR, "has space")]).expect_err("refused");

        assert!(matches!(error, ConfigError::Token));
        assert!(!error.to_string().contains(leaked), "{error}");
        assert!(!format!("{error:?}").contains(leaked), "{error:?}");
    }

    #[test]
    fn the_config_debug_shows_no_token() -> Result<(), ConfigError> {
        let config = config(&[(ACCOUNT_VAR, "abc"), (TOKEN_VAR, SECRET)])?;

        assert!(!format!("{config:?}").contains(SECRET));
        Ok(())
    }
}
