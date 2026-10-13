//! The defaults and argument readers Tier 2's and Tier 3's flags share.

use std::path::Path;
use std::time::Duration;

/// The cost cap when `--max-cost-usd` is not given, in US dollars.
/// A full run of the repository's suite is capped at $25 (Ryan, wi-92bc1b); Tier 3 shares it.
pub(crate) const DEFAULT_MAX_COST_USD: f64 = 25.0;
/// The time limit when `--timeout` is not given.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3600);
/// Runs at once when `--concurrency` is not given.
pub(crate) const DEFAULT_CONCURRENCY: u8 = 4;

/// A `--max-cost-usd` value: a finite amount above zero.
pub(crate) fn cost_cap(text: &str) -> Result<f64, String> {
    match text.parse::<f64>() {
        Ok(usd) if usd.is_finite() && usd > 0.0 => Ok(usd),
        Ok(_) | Err(_) => Err(format!(
            "`{text}` is not an amount of US dollars above zero"
        )),
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
