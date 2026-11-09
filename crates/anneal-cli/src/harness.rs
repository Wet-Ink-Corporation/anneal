//! The Claude Code harness adapter: every environment variable Claude Code sets for the
//! plugin's hooks, commands and status line, by name, with what each one means.
//!
//! This is the one seam between the CLI and the harness that runs it (CLI-13): the rest of
//! the crate asks this module for a variable's name and reads it through its injected
//! `lookup`, and never spells a `CLAUDE_*` string itself (a test in this module scans the
//! sources for one). anneal's own variables (`ANNEAL_*`, `WEIGHIN_BIN`) and third-party ones
//! (`TMPDIR`, `SSL_CERT_FILE`, the proxy variables) stay with the code that reads them.
//!
//! Plugin options: Claude Code (checked in 2.1.296) gives a hook each `userConfig` value,
//! sensitive ones included, as `CLAUDE_PLUGIN_OPTION_<KEY>`, the key upper-cased with every
//! character outside `[A-Za-z0-9_]` turned into `_`.

/// `userConfig.cf_account_id`: the Cloudflare account that runs Clef; empty means Clef is off.
pub const ACCOUNT_VAR: &str = "CLAUDE_PLUGIN_OPTION_CF_ACCOUNT_ID";

/// `userConfig.cf_api_token` (sensitive): the Workers AI token.
pub const TOKEN_VAR: &str = "CLAUDE_PLUGIN_OPTION_CF_API_TOKEN";

/// The `userConfig` key holding the Clef mode, which [`MODE_VAR`] carries.
pub const MODE_KEY: &str = "clef_mode";

/// `userConfig.clef_mode`: `off`, `shadow` or `enforce`, with per-point overrides.
pub const MODE_VAR: &str = "CLAUDE_PLUGIN_OPTION_CLEF_MODE";

/// The auto-compact window in tokens, which Claude Code clamps to 100,000..=1,000,000.
pub const AUTO_COMPACT_WINDOW_VAR: &str = "CLAUDE_CODE_AUTO_COMPACT_WINDOW";

/// Given to a `SessionStart` hook: a shell script Claude Code sources before each later Bash
/// command.
pub const ENV_FILE_VAR: &str = "CLAUDE_ENV_FILE";

/// The plugin's root directory, the one holding `libexec/` and `skills/`.
pub const PLUGIN_ROOT_VAR: &str = "CLAUDE_PLUGIN_ROOT";

/// The project's root directory, the one Claude Code was started in.
pub const PROJECT_DIR_VAR: &str = "CLAUDE_PROJECT_DIR";

#[cfg(test)]
mod tests;
