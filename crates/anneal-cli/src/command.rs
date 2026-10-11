//! The closed set of top-level commands and what a green result from each one claims.

/// A top-level `anneal` command. The set is closed: adding one means adding a variant, and
/// every exhaustive `match` (exit codes, claims, dispatch) then fails to compile until it is
/// handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CommandName {
    /// `anneal ledger`
    Ledger,
    /// `anneal task`
    Task,
    /// `anneal plan`
    Plan,
    /// `anneal lint`
    Lint,
    /// `anneal dag`
    Dag,
    /// `anneal floor-guard`
    FloorGuard,
    /// `anneal verdict`
    Verdict,
    /// `anneal watch`
    Watch,
    /// `anneal ship`
    Ship,
    /// `anneal rollout`
    Rollout,
    /// `anneal perf`
    Perf,
    /// `anneal brief`
    Brief,
    /// `anneal decide`
    Decide,
    /// `anneal calibrate`
    Calibrate,
    /// `anneal eval`
    Eval,
    /// `anneal statusline`
    Statusline,
}

/// What a green (exit 0) result from a command establishes, and what it does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Claim {
    /// What a green result proves.
    pub proves: &'static str,
    /// What a green result does NOT prove.
    pub does_not_prove: &'static str,
}

impl CommandName {
    /// Every command, in the order `--help` and `docs/exit-codes.md` list them.
    pub const ALL: [Self; 16] = [
        Self::Ledger,
        Self::Task,
        Self::Plan,
        Self::Lint,
        Self::Dag,
        Self::FloorGuard,
        Self::Verdict,
        Self::Watch,
        Self::Ship,
        Self::Rollout,
        Self::Perf,
        Self::Brief,
        Self::Decide,
        Self::Calibrate,
        Self::Eval,
        Self::Statusline,
    ];

    /// The name as typed on the command line.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ledger => "ledger",
            Self::Task => "task",
            Self::Plan => "plan",
            Self::Lint => "lint",
            Self::Dag => "dag",
            Self::FloorGuard => "floor-guard",
            Self::Verdict => "verdict",
            Self::Watch => "watch",
            Self::Ship => "ship",
            Self::Rollout => "rollout",
            Self::Perf => "perf",
            Self::Brief => "brief",
            Self::Decide => "decide",
            Self::Calibrate => "calibrate",
            Self::Eval => "eval",
            Self::Statusline => "statusline",
        }
    }

    /// What a green result from this command proves and does not prove.
    #[must_use]
    pub const fn claim(self) -> Claim {
        Claim {
            proves: self.proves(),
            does_not_prove: self.does_not_prove(),
        }
    }

    const fn proves(self) -> &'static str {
        match self {
            Self::Ledger => "the ledger events read or written match the event schema",
            Self::Task => "the task's tests ran green",
            Self::Plan => "the plan is approved and current with its inputs",
            Self::Lint => "the checked files have no error under anneal's lint rules",
            Self::Dag => "the task graph has no cycle and no dangling dependency",
            Self::FloorGuard => "no quality floor (coverage, lint level, test count) was lowered",
            Self::Verdict => "the review verdict is well-formed and backed by findings",
            Self::Watch => "the pull request, CI run or deploy reached green within the timeout",
            Self::Ship => "the change meets the rule for skipping a full review",
            Self::Rollout => "the rollout metrics are within their bounds",
            Self::Perf => "the measured change is not a regression beyond run-to-run noise",
            Self::Brief => "the brief was built from the given commit range",
            Self::Decide => "the decision was obtained and recorded",
            Self::Calibrate => "the calibration run completed and was recorded",
            Self::Eval => "skill routing accuracy is at or above its bound on the eval set",
            Self::Statusline => "the status line was rendered",
        }
    }

    const fn does_not_prove(self) -> &'static str {
        match self {
            Self::Ledger => "that the events are true, or that the work they record was done well",
            Self::Task => "that the tests cover the task's acceptance criteria",
            Self::Plan => "that the plan is correct or complete",
            Self::Lint => {
                "that the content is right, or that its warnings and review items were addressed"
            }
            Self::Dag => "that the dependencies are the right ones",
            Self::FloorGuard => "that the floors are high enough",
            Self::Verdict => "that the reviewer was right",
            Self::Watch => "that it stays green, or that its checks test the right things",
            Self::Ship => "that the change is free of defects",
            Self::Rollout => "that the metrics capture what users experience",
            Self::Perf => "performance on hardware or workloads that were not measured",
            Self::Brief => "that the brief is complete or its summary accurate",
            Self::Decide => "that the decision is right",
            Self::Calibrate => "that the decision point is calibrated well enough to enforce",
            Self::Eval => "routing accuracy on prompts outside the eval set",
            Self::Statusline => "that the state it shows is current",
        }
    }
}

impl std::fmt::Display for CommandName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::CommandName;
    use std::collections::BTreeSet;

    #[test]
    fn every_command_has_a_distinct_name_and_a_does_not_prove_statement() {
        let names: BTreeSet<&str> = CommandName::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(names.len(), CommandName::ALL.len(), "names are distinct");
        for command in CommandName::ALL {
            assert!(
                !command.claim().does_not_prove.is_empty(),
                "{command} states what it does not prove"
            );
        }
    }
}
