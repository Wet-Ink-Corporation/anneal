//! Numeric thresholds on a line (`key = 80`, `"key": 80`, `key: 80`, `--key=80`, `--key 80`),
//! and whether a change of one loosens a floor.

/// A `key` and its numeric `value` on one line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Pair<'a> {
    pub(super) key: &'a str,
    pub(super) value: f64,
}

/// Which way a threshold loosens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    /// A coverage floor: lowering loosens.
    Coverage,
    /// A timeout: raising loosens.
    Timeout,
    /// A ratchet `max*` value: raising loosens.
    RatchetMax,
    /// A ratchet `min*` value: lowering loosens.
    RatchetMin,
}

impl Kind {
    /// True when moving from `old` to `new` loosens the floor.
    pub(super) fn loosened(self, old: f64, new: f64) -> bool {
        match self {
            Self::Coverage | Self::RatchetMin => new < old,
            Self::Timeout | Self::RatchetMax => new > old,
        }
    }
}

/// Coverage keys wherever they appear.
const COVERAGE_KEYS: [&str; 3] = ["fail_under", "fail-under", "min_coverage"];
/// Keys that are coverage floors only under jest's `coverageThreshold`.
const COVERAGE_THRESHOLD_CHILDREN: [&str; 4] = ["lines", "branches", "functions", "statements"];
/// Timeout keys.
const TIMEOUT_KEYS: [&str; 4] = ["timeout", "timeout_ms", "testTimeout", "timeout-minutes"];

/// The kind of threshold `key` is in the file at `path`; `coverage_threshold` says whether the
/// file mentions `coverageThreshold`. In a `*ratchet*.json` file (a placeholder until #24
/// defines the ratchet format) every `max*`/`min*` key is a ratchet value.
pub(super) fn kind(path: &str, key: &str, coverage_threshold: bool) -> Option<Kind> {
    let key = key.trim_start_matches('-');
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    let json = name.rsplit_once('.').is_some_and(|(_, ext)| ext == "json");
    if json && name.contains("ratchet") {
        let lower = key.to_ascii_lowercase();
        if lower.starts_with("max") {
            return Some(Kind::RatchetMax);
        }
        if lower.starts_with("min") {
            return Some(Kind::RatchetMin);
        }
    }
    let coverage = COVERAGE_KEYS.contains(&key)
        || key.ends_with("-fail-under")
        || key.ends_with("_fail_under")
        || (coverage_threshold && COVERAGE_THRESHOLD_CHILDREN.contains(&key));
    if coverage {
        return Some(Kind::Coverage);
    }
    TIMEOUT_KEYS.contains(&key).then_some(Kind::Timeout)
}

/// Every `key` followed by a separator (`=`, `:`, or a space after a `--flag`) and a number.
/// Quotes around the key or the number are skipped.
pub(super) fn pairs(text: &str) -> Vec<Pair<'_>> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(is_key_char) {
        let Some(from) = rest.get(start..) else { break };
        let end = from.find(|c: char| !is_key_char(c)).unwrap_or(from.len());
        let (key, after) = from.split_at_checked(end).unwrap_or((from, ""));
        if let Some(value) = value_after(key, after)
            && !key.chars().all(|c| c.is_ascii_digit() || c == '.')
        {
            found.push(Pair { key, value });
        }
        rest = after;
    }
    found
}

/// The number after `key`'s separator in `after`, if there is one.
fn value_after(key: &str, after: &str) -> Option<f64> {
    let after = after.strip_prefix(['"', '\'']).unwrap_or(after);
    let trimmed = after.trim_start_matches([' ', '\t']);
    let spaced = trimmed.len() < after.len();
    let value = if let Some(value) = trimmed.strip_prefix(['=', ':']) {
        value
    } else if key.starts_with("--") && spaced {
        trimmed
    } else {
        return None;
    };
    let value = value.trim_start_matches([' ', '\t']);
    let value = value.strip_prefix(['"', '\'']).unwrap_or(value);
    let end = value
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(value.len());
    value.get(..end)?.parse().ok()
}

fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')
}
