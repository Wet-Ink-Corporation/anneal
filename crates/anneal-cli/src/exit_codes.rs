//! The exit-code table: the one place exit codes are declared.
//!
//! Codes have a per-command meaning (`1` is "cycle" for `dag` but "test red" for `task`), so
//! each command has its own table, plus a few global codes that mean the same everywhere. The
//! table is rendered into `--help` and into `docs/exit-codes.md` by [`render`]; a test fails
//! when the committed markdown drifts from it.
//!
//! The rule behind every row: "2 never reads as 0". Only success exits 0.

pub mod render;

use crate::command::CommandName;

/// One documented exit code and what it means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Code {
    value: u8,
    meaning: &'static str,
}

impl Code {
    /// Declares a code. Private: every code is a constant in this module, so the table and the
    /// generated docs are the only place codes come from.
    #[must_use]
    const fn new(value: u8, meaning: &'static str) -> Self {
        Self { value, meaning }
    }

    /// The process exit status.
    #[must_use]
    pub const fn value(self) -> u8 {
        self.value
    }

    /// What the code means, as documented.
    #[must_use]
    pub const fn meaning(self) -> &'static str {
        self.meaning
    }

    /// True only for exit status 0.
    #[must_use]
    pub const fn is_success(self) -> bool {
        self.value == 0
    }
}

impl From<Code> for std::process::ExitCode {
    fn from(code: Code) -> Self {
        Self::from(code.value)
    }
}

/// A command's exit-code table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandCodes {
    /// The command the table belongs to.
    pub command: CommandName,
    /// The subcommands the table applies to (`"check|frontier"`), or `""` for all of them.
    pub scope: &'static str,
    /// The codes, in ascending order.
    pub codes: &'static [Code],
}

/// Usage error: bad arguments or an unknown command. Nothing is written on stdout.
///
/// Kept at clap's 2 because Claude Code reads a hook's exit 2 as "block", so a mis-invoked guard fails
/// closed; a command's own 2 always comes with a JSON envelope on stdout, a usage error never.
pub const USAGE: Code = Code::new(2, "usage error: bad arguments; nothing on stdout");
/// The command exists in the tree but is not implemented yet.
pub const NOT_IMPLEMENTED: Code = Code::new(64, "not implemented yet");
/// The command could not run: git missing, unreadable snapshot, and the like.
pub const INTERNAL: Code = Code::new(70, "internal error: the command could not run");
/// stdout or stderr could not be written, so the result was not delivered.
pub const OUTPUT: Code = Code::new(74, "output error: stdout or stderr could not be written");

/// Codes that mean the same for every command. Command tables never reuse 64, 70 or 74.
pub const GLOBAL: &[Code] = &[USAGE, NOT_IMPLEMENTED, INTERNAL, OUTPUT];

/// Success, for commands whose 0 needs no more specific wording.
pub const OK: Code = Code::new(0, "ok");

/// `ledger`: an event does not match the ledger schema.
pub const LEDGER_SCHEMA_ERROR: Code = Code::new(2, "schema error");
/// `grill`: the design tree is invalid (a `depends_on` cycle, a dangling dependency, a
/// duplicate id, a bad field or bound); nothing is recorded.
pub const GRILL_INVALID: Code = Code::new(1, "invalid design tree");
/// `grill confirm`: a question is still open, so nothing is recorded.
pub const GRILL_OPEN: Code = Code::new(3, "confirm: a question is still open");
/// `task done`: the task's tests are green.
pub const TASK_GREEN: Code = Code::new(0, "tests green");
/// `task done`: a test is red (or ran past its timeout); only the red `test_run` is recorded.
pub const TASK_TEST_RED: Code = Code::new(1, "test red");
/// `task done`: the test command could not be started; nothing is recorded.
pub const TASK_NOT_STARTED: Code = Code::new(2, "the test command could not be started");
/// `plan approve|check`: the plan is approved and current.
pub const PLAN_APPROVED: Code = Code::new(0, "approved and current");
/// `plan approve|check`: the plan is not approved.
pub const PLAN_NOT_APPROVED: Code = Code::new(3, "not approved");
/// `plan approve|check`: the plan was approved but its inputs changed since.
pub const PLAN_STALE: Code = Code::new(4, "stale: inputs changed since approval");
/// `mode build|diagnose|off`: the mode was recorded (a dirty tree is recorded too, with a
/// finding).
pub const MODE_RECORDED: Code = Code::new(0, "mode recorded");
/// `repro record|leftovers`: record appended `repro_red` (the command exited non-zero);
/// leftovers found no `[DEBUG-<id>]` tag.
pub const REPRO_OK: Code = Code::new(
    0,
    "record: red, and `repro_red` recorded; leftovers: no `[DEBUG-<id>]` tag left",
);
/// `repro leftovers`: a `[DEBUG-<id>]` tag is left in a file git knows of.
pub const REPRO_LEFTOVERS: Code = Code::new(1, "leftovers: a `[DEBUG-<id>]` tag is left");
/// `repro record`: the command could not be started; nothing is recorded.
pub const REPRO_NOT_STARTED: Code = Code::new(
    2,
    "record: the command could not be started; nothing recorded",
);
/// `repro record`: the command exited 0, so it reproduces nothing; nothing is recorded.
pub const REPRO_NOT_RED: Code =
    Code::new(3, "record: not red, the command exited 0; nothing recorded");
/// `repro record`: the command ran past its timeout and was killed; a hang is not a
/// reproduction, so nothing is recorded.
pub const REPRO_TIMED_OUT: Code = Code::new(
    4,
    "record: timed out, killed; a hang is not a reproduction, nothing recorded",
);
/// `lint`: no error findings (warnings and review items are listed but do not fail).
pub const LINT_CLEAN: Code = Code::new(0, "no errors");
/// `lint`: at least one error finding.
pub const LINT_FINDINGS: Code = Code::new(1, "error findings");
/// `dag check|frontier`: the graph has a cycle.
pub const DAG_CYCLE: Code = Code::new(1, "cycle");
/// `dag check|frontier`: a dependency names a task that does not exist.
pub const DAG_DANGLING: Code = Code::new(2, "dangling dependency");
/// `floor-guard`: no floor was lowered.
pub const FLOOR_HELD: Code = Code::new(0, "no floor lowered");
/// `floor-guard`: a floor was lowered.
pub const FLOOR_LOWERED: Code = Code::new(1, "a floor was lowered");
/// `floor-guard`: a measuring tool failed.
pub const FLOOR_TOOL_ERROR: Code = Code::new(2, "tool error");
/// `verdict set|valid`: the verdict is valid.
pub const VERDICT_VALID: Code = Code::new(0, "valid");
/// `verdict set|valid`: the verdict is invalid.
pub const VERDICT_INVALID: Code = Code::new(1, "invalid");
/// `verdict set|valid`: every finding is noise.
pub const VERDICT_NOISE_ONLY: Code = Code::new(3, "noise-only");
/// `watch pr|ci|deploy`: the watched target is green.
pub const WATCH_GREEN: Code = Code::new(0, "green");
/// `watch pr|ci|deploy`: merge conflict.
pub const WATCH_CONFLICT: Code = Code::new(2, "merge conflict");
/// `watch pr|ci|deploy`: unresolved review threads.
pub const WATCH_THREADS: Code = Code::new(3, "unresolved review threads");
/// `watch pr|ci|deploy`: failing checks.
pub const WATCH_CHECKS: Code = Code::new(4, "failing checks");
/// `watch pr|ci|deploy`: timed out before a result.
pub const WATCH_TIMEOUT: Code = Code::new(5, "timeout");
/// `watch pr|ci|deploy`: a gate failed.
pub const WATCH_GATE: Code = Code::new(6, "gate failed");
/// `watch pr|ci|deploy`: the status query failed.
pub const WATCH_QUERY: Code = Code::new(7, "query failed");
/// `ship skip-rule`: the change may skip full review.
pub const SHIP_SKIP: Code = Code::new(0, "skip full review");
/// `ship skip-rule`: the change needs a full review.
pub const SHIP_FULL_REVIEW: Code = Code::new(1, "full review");
/// `rollout decide`: go.
pub const ROLLOUT_GO: Code = Code::new(0, "go");
/// `rollout decide`: hold.
pub const ROLLOUT_HOLD: Code = Code::new(1, "hold");
/// `rollout decide`: roll back.
pub const ROLLOUT_ROLLBACK: Code = Code::new(2, "rollback");
/// `perf noise`: keep the change.
pub const PERF_KEEP: Code = Code::new(0, "keep");
/// `perf noise`: revert the change.
pub const PERF_REVERT: Code = Code::new(1, "revert");
/// `brief build`: the commit range is invalid.
pub const BRIEF_BAD_RANGE: Code = Code::new(2, "bad range");
/// `decide`: Clef could not be reached.
pub const DECIDE_CLEF_UNREACHABLE: Code = Code::new(5, "Clef unreachable");
/// `eval routing`: accuracy is below its bound.
pub const EVAL_BELOW_BOUND: Code = Code::new(1, "below bound");
/// `eval routing`: a fixture or skill is invalid or missing, so accuracy cannot be trusted.
pub const EVAL_INVALID: Code = Code::new(2, "invalid fixtures or skills");
/// `hook`: the hook ran (a guard's deny is a decision in its output), or failed open with its
/// failure noted in its output or on stderr.
pub const HOOK_DONE: Code = Code::new(
    0,
    "ran (a guard's deny is on stdout), or failed open with the failure noted",
);
/// `hook`: a guard (or a hook passed `--fail-closed`) could not deliver its output, so it
/// failed closed. Claude Code reads 2 as "block".
pub const HOOK_BLOCKED: Code = Code::new(2, "a guard failed closed: its output was not written");

impl CommandCodes {
    /// The exit-code table of `command`.
    #[must_use]
    pub const fn of(command: CommandName) -> Self {
        let (scope, codes): (&'static str, &'static [Code]) = match command {
            CommandName::Ledger => ("", &[OK, LEDGER_SCHEMA_ERROR]),
            CommandName::Grill => ("", &[OK, GRILL_INVALID, GRILL_OPEN]),
            CommandName::Task => ("done", &[TASK_GREEN, TASK_TEST_RED, TASK_NOT_STARTED]),
            CommandName::Plan => (
                "approve|check",
                &[PLAN_APPROVED, PLAN_NOT_APPROVED, PLAN_STALE],
            ),
            CommandName::Mode => ("", &[MODE_RECORDED]),
            // No scope: it would widen the help's title column; each meaning names its
            // subcommand instead.
            CommandName::Repro => (
                "",
                &[
                    REPRO_OK,
                    REPRO_LEFTOVERS,
                    REPRO_NOT_STARTED,
                    REPRO_NOT_RED,
                    REPRO_TIMED_OUT,
                ],
            ),
            CommandName::Lint => ("", &[LINT_CLEAN, LINT_FINDINGS]),
            CommandName::Dag => ("check|frontier", &[OK, DAG_CYCLE, DAG_DANGLING]),
            CommandName::FloorGuard => ("", &[FLOOR_HELD, FLOOR_LOWERED, FLOOR_TOOL_ERROR]),
            CommandName::Verdict => (
                "set|valid",
                &[VERDICT_VALID, VERDICT_INVALID, VERDICT_NOISE_ONLY],
            ),
            CommandName::Watch => (
                "pr|ci|deploy",
                &[
                    WATCH_GREEN,
                    WATCH_CONFLICT,
                    WATCH_THREADS,
                    WATCH_CHECKS,
                    WATCH_TIMEOUT,
                    WATCH_GATE,
                    WATCH_QUERY,
                ],
            ),
            CommandName::Ship => ("skip-rule", &[SHIP_SKIP, SHIP_FULL_REVIEW]),
            CommandName::Rollout => ("decide", &[ROLLOUT_GO, ROLLOUT_HOLD, ROLLOUT_ROLLBACK]),
            CommandName::Perf => ("noise", &[PERF_KEEP, PERF_REVERT]),
            CommandName::Brief => ("build", &[OK, BRIEF_BAD_RANGE]),
            CommandName::Decide => ("", &[OK, DECIDE_CLEF_UNREACHABLE]),
            CommandName::Eval => ("routing", &[OK, EVAL_BELOW_BOUND, EVAL_INVALID]),
            CommandName::Calibrate | CommandName::Statusline => ("", &[OK]),
            CommandName::Hook => ("", &[HOOK_DONE, HOOK_BLOCKED]),
        };
        Self {
            command,
            scope,
            codes,
        }
    }

    /// The command and its scope as typed: `"dag check|frontier"`, or `"ledger"`.
    #[must_use]
    pub fn title(&self) -> String {
        if self.scope.is_empty() {
            self.command.as_str().to_owned()
        } else {
            format!("{} {}", self.command, self.scope)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CommandCodes, GLOBAL, USAGE};
    use crate::command::CommandName;

    #[test]
    fn each_command_table_has_exactly_one_success_code() {
        for command in CommandName::ALL {
            let table = CommandCodes::of(command);
            let zeros = table.codes.iter().filter(|c| c.is_success()).count();
            assert_eq!(zeros, 1, "{command} has one 0 row");
        }
    }

    #[test]
    fn codes_are_ascending_and_unique_within_a_command() {
        for command in CommandName::ALL {
            let values: Vec<u8> = CommandCodes::of(command)
                .codes
                .iter()
                .map(|c| c.value())
                .collect();
            assert!(
                values.windows(2).all(|w| matches!(w, [a, b] if a < b)),
                "{command} codes ascend strictly: {values:?}"
            );
        }
    }

    #[test]
    fn command_codes_never_reuse_a_global_code_other_than_usage() {
        for command in CommandName::ALL {
            for code in CommandCodes::of(command).codes {
                let clash = GLOBAL
                    .iter()
                    .any(|g| *g != USAGE && g.value() == code.value());
                assert!(!clash, "{command} reuses global code {}", code.value());
            }
        }
    }

    #[test]
    fn no_global_code_is_success() {
        assert!(
            GLOBAL.iter().all(|c| !c.is_success()),
            "global codes are all failures"
        );
    }

    #[test]
    fn exit_code_conversion_keeps_the_value() {
        let code = super::NOT_IMPLEMENTED;
        assert_eq!(
            std::process::ExitCode::from(code),
            std::process::ExitCode::from(64),
            "64 maps to exit status 64"
        );
    }
}
