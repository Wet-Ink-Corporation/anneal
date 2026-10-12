//! The plugin's `userConfig` as hooks see it: the account, the token and the Clef mode.
//!
//! Claude Code (checked in 2.1.296) gives a hook each `userConfig` value, sensitive ones
//! included, as `CLAUDE_PLUGIN_OPTION_<KEY>`, the key upper-cased with every character outside
//! `[A-Za-z0-9_]` turned into `_`. An empty account means Clef is off. The mode is
//! `<global>[,<point>=<mode>]...`, each mode `off`, `shadow` or `enforce`, spaces around the
//! parts ignored; empty or unset is `shadow`, the manifest's default.
//!
//! [`ENDPOINT_VAR`] (`ANNEAL_CLEF_URL`) replaces the Workers AI accounts URL for tests. Only a
//! debug build reads it (a release build ignores it entirely), and only as `http://` on
//! `127.0.0.1` or `[::1]`, so it cannot send the token off the machine; any other value is
//! [`ConfigError::Endpoint`].

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

/// Test-only override of the Workers AI accounts URL; loopback `http://` only, debug builds
/// only.
pub const ENDPOINT_VAR: &str = "ANNEAL_CLEF_URL";

/// Whether [`ENDPOINT_VAR`] is read at all: only in debug builds (the test binaries), never in
/// a release build, so a shipped `anneal` cannot be pointed at a cleartext listener.
const HONOUR_ENDPOINT: bool = cfg!(debug_assertions);

/// The hosts [`ENDPOINT_VAR`] may name: loopback literals only, since `localhost` resolves
/// through `/etc/hosts` and NSS.
const LOOPBACK_HOSTS: [&str; 2] = ["127.0.0.1", "[::1]"];

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
    /// The endpoint override is not a loopback `http://` URL.
    #[error("{ENDPOINT_VAR} must be http:// on 127.0.0.1 or [::1], with an optional port and path")]
    Endpoint,
}

/// The account, token and mode a decision runs with.
#[derive(Debug)]
pub struct ClefConfig {
    account: Option<String>,
    token: Option<ApiToken>,
    global: Mode,
    overrides: BTreeMap<Point, Mode>,
    endpoint: Option<String>,
}

impl ClefConfig {
    /// Reads the configuration through `lookup`; empty values count as unset.
    ///
    /// # Errors
    /// [`ConfigError`] for a value that is not UTF-8, an account that is not alphanumeric, a
    /// token unfit for a header, a mode that does not parse, or an endpoint override that is
    /// not a loopback `http://` URL.
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
        let endpoint = endpoint_override(HONOUR_ENDPOINT, || read(ENDPOINT_VAR))?;
        Ok(Self {
            account,
            token,
            global,
            overrides,
            endpoint,
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

    /// The test endpoint that replaces the Workers AI accounts URL, if [`ENDPOINT_VAR`] is set:
    /// a loopback `http://` base without a trailing `/`.
    #[must_use]
    pub fn endpoint(&self) -> Option<&str> {
        self.endpoint.as_deref()
    }

    /// The configured mode of `point`: its override, else the global mode.
    #[must_use]
    pub fn mode(&self, point: Point) -> Mode {
        self.overrides.get(&point).copied().unwrap_or(self.global)
    }
}

/// The checked endpoint override when `honoured`; otherwise `None` without calling `read`, so
/// a release build never looks at the variable.
fn endpoint_override(
    honoured: bool,
    read: impl FnOnce() -> Result<Option<String>, ConfigError>,
) -> Result<Option<String>, ConfigError> {
    if !honoured {
        return Ok(None);
    }
    read()?
        .map(|value| loopback_base(&value).ok_or(ConfigError::Endpoint))
        .transpose()
}

/// `value` without one trailing `/`, when it is `http://`, a loopback host, an optional port
/// (1 to 5 digits, at most 65535) and an optional path of `[A-Za-z0-9._~/-]`; else `None`.
fn loopback_base(value: &str) -> Option<String> {
    let rest = value.strip_prefix("http://")?;
    let (authority, path) = rest
        .find('/')
        .map_or((rest, ""), |slash| rest.split_at(slash));
    let port = LOOPBACK_HOSTS
        .into_iter()
        .find_map(|host| authority.strip_prefix(host))?;
    let port_ok = port.is_empty()
        || port.strip_prefix(':').is_some_and(|digits| {
            (1..=5).contains(&digits.len())
                && digits.bytes().all(|byte| byte.is_ascii_digit())
                && digits.parse::<u16>().is_ok()
        });
    let path_ok = path
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"._~/-".contains(&byte));
    (port_ok && path_ok).then(|| value.strip_suffix('/').unwrap_or(value).to_owned())
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
    use std::cell::Cell;
    use std::collections::BTreeMap;
    use std::ffi::OsString;

    use super::{
        ACCOUNT_VAR, ClefConfig, ConfigError, ENDPOINT_VAR, MODE_VAR, Mode, TOKEN_VAR,
        endpoint_override,
    };
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
    fn without_an_endpoint_override_there_is_none() -> Result<(), ConfigError> {
        assert_eq!(config(&[])?.endpoint(), None);
        assert_eq!(config(&[(ENDPOINT_VAR, "")])?.endpoint(), None);
        Ok(())
    }

    #[test]
    fn a_loopback_http_endpoint_is_taken_without_a_trailing_slash() -> Result<(), ConfigError> {
        for (value, base) in [
            ("http://127.0.0.1:8080", "http://127.0.0.1:8080"),
            ("http://127.0.0.1:0/", "http://127.0.0.1:0"),
            ("http://[::1]:9/accounts", "http://[::1]:9/accounts"),
            (
                "http://127.0.0.1:65535/a/b-c_d.e/",
                "http://127.0.0.1:65535/a/b-c_d.e",
            ),
        ] {
            assert_eq!(
                config(&[(ENDPOINT_VAR, value)])?.endpoint(),
                Some(base),
                "{value}"
            );
        }
        Ok(())
    }

    #[test]
    fn an_endpoint_off_the_loopback_host_is_refused() {
        for value in [
            "https://api.cloudflare.com/client/v4/accounts",
            "https://127.0.0.1:8080",
            "http://127.0.0.2",
            "http://127.0.0.1.example.com",
            "http://localhost",
            "http://localhost:8080/accounts",
            "http://localhost.example.com/",
            "http://localhost@example.com",
            "http://user@127.0.0.1",
            "http://example.com/127.0.0.1",
            "http://127.0.0.1:99999",
            "http://127.0.0.1:",
            "http://127.0.0.1:8x",
            "http://[::1",
            "http://127.0.0.1/a b",
            "http://127.0.0.1/a?b",
            "http://127.0.0.1/a#b",
            "ftp://127.0.0.1",
            "127.0.0.1:8080",
        ] {
            assert!(
                matches!(config(&[(ENDPOINT_VAR, value)]), Err(ConfigError::Endpoint)),
                "{value:?} is refused"
            );
        }
    }

    #[test]
    fn a_release_build_ignores_the_endpoint_override_without_reading_it() {
        let read = Cell::new(false);

        let endpoint = endpoint_override(false, || {
            read.set(true);
            Ok(Some("https://example.com/not-loopback".to_owned()))
        });

        assert!(matches!(endpoint, Ok(None)), "{endpoint:?}");
        assert!(!read.get(), "the variable is not read");
    }

    #[test]
    fn a_debug_build_reads_and_checks_the_endpoint_override() {
        let taken = endpoint_override(true, || Ok(Some("http://127.0.0.1:8/".to_owned())));
        let refused = endpoint_override(true, || Ok(Some("https://example.com".to_owned())));

        assert_eq!(taken.ok(), Some(Some("http://127.0.0.1:8".to_owned())));
        assert!(matches!(refused, Err(ConfigError::Endpoint)), "{refused:?}");
    }

    #[test]
    fn the_endpoint_variable_is_anneals_own() {
        assert_eq!(ENDPOINT_VAR, "ANNEAL_CLEF_URL");
    }

    #[test]
    fn the_config_debug_shows_no_token() -> Result<(), ConfigError> {
        let config = config(&[(ACCOUNT_VAR, "abc"), (TOKEN_VAR, SECRET)])?;

        assert!(!format!("{config:?}").contains(SECRET));
        Ok(())
    }
}
