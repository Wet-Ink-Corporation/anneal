//! Each hook's metadata: why it exists, so scaffolding hooks can be re-tested and retired as
//! models improve.
//!
//! A closed table keyed by [`HookName`], so a new hook cannot compile without its row;
//! `tests/it/hooks_manifest.rs` keeps it in agreement with `plugin/hooks/hooks.json`.

use super::HookName;

/// The Claude Code event a hook entry is declared under in `hooks.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    /// `SessionStart`
    SessionStart,
    /// `UserPromptSubmit`
    UserPromptSubmit,
    /// `PreToolUse`
    PreToolUse,
    /// `PostToolBatch`
    PostToolBatch,
    /// `Stop`
    Stop,
    /// `SubagentStart`
    SubagentStart,
    /// `SubagentStop`
    SubagentStop,
    /// `PreCompact`
    PreCompact,
}

impl HookEvent {
    /// The event's name, as `hooks.json` spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SessionStart => "SessionStart",
            Self::UserPromptSubmit => "UserPromptSubmit",
            Self::PreToolUse => "PreToolUse",
            Self::PostToolBatch => "PostToolBatch",
            Self::Stop => "Stop",
            Self::SubagentStart => "SubagentStart",
            Self::SubagentStop => "SubagentStop",
            Self::PreCompact => "PreCompact",
        }
    }
}

/// The entry's `onFailure`: what Claude Code does when the hook crashes or times out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnFailure {
    /// `continue`: fail open.
    Continue,
    /// `block`: fail closed, as a guard does.
    Block,
}

impl OnFailure {
    /// The policy as `hooks.json` spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Continue => "continue",
            Self::Block => "block",
        }
    }
}

/// Why a hook exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Helps a model generation; re-test it when the default model changes and retire it when
    /// it adds nothing.
    Scaffolding,
    /// Enforces a gate.
    Guard,
}

impl Role {
    /// The role's name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Scaffolding => "scaffolding",
            Self::Guard => "guard",
        }
    }
}

/// One hook's row. Its id is the [`HookName`] it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HookMetadata {
    /// The event the `hooks.json` entry is declared under.
    pub event: HookEvent,
    /// The entry's matcher, exactly (empty when it has none).
    pub matcher: &'static str,
    /// `sha256:` and the first 16 hex digits of SHA-256 over `<event>\n<matcher>\n<the hook
    /// object as compact JSON, keys sorted>`; it changes whenever the entry does.
    pub fingerprint: &'static str,
    /// The entry's `onFailure`.
    pub on_failure: OnFailure,
    /// Why the hook exists.
    pub role: Role,
    /// The model generation the hook was added for, such as `claude-5`.
    pub model_generation: &'static str,
    /// Where the hook is specified: blueprint section and story.
    pub source: &'static str,
}

impl HookName {
    /// The hook's metadata row.
    #[must_use]
    pub const fn metadata(self) -> HookMetadata {
        match self {
            Self::Bootstrap => BOOTSTRAP,
            Self::Guard => GUARD,
            Self::MergeGate => MERGE_GATE,
            Self::Preconditions => PRECONDITIONS,
            Self::FastCheck => FAST_CHECK,
            Self::Completeness => COMPLETENESS,
            Self::Status => STATUS,
            Self::Route => ROUTE,
            Self::RuledBefore => RULED_BEFORE,
            Self::InjectionScan => INJECTION_SCAN,
            Self::ContextMonitor => CONTEXT_MONITOR,
            Self::Handoff => HANDOFF,
            Self::Brief => BRIEF,
            Self::StandingOrders => STANDING_ORDERS,
        }
    }
}

/// The generation every hook so far was added for.
const CLAUDE_5: &str = "claude-5";

const BOOTSTRAP: HookMetadata = HookMetadata {
    event: HookEvent::SessionStart,
    matcher: "startup|clear|compact|resume",
    fingerprint: "sha256:856794ce16dcf8fd",
    on_failure: OnFailure::Continue,
    role: Role::Scaffolding,
    model_generation: CLAUDE_5,
    source: "Blueprint: Hooks (bootstrap), Feature catalog #1, skill rules 4, 15, 19 and 21; \
             story #13",
};

const GUARD: HookMetadata = HookMetadata {
    event: HookEvent::PreToolUse,
    matcher: "Bash",
    fingerprint: "sha256:df2690e5abee2911",
    on_failure: OnFailure::Block,
    role: Role::Guard,
    model_generation: CLAUDE_5,
    source: "Blueprint: Hooks (guard), Gate escalation, skill rule 15; story #14",
};

const MERGE_GATE: HookMetadata = HookMetadata {
    event: HookEvent::PreToolUse,
    matcher: "Bash",
    fingerprint: "sha256:2ec04a288f2021d2",
    on_failure: OnFailure::Block,
    role: Role::Guard,
    model_generation: CLAUDE_5,
    source: "Blueprint: Hooks (merge-gate), Feature catalog #24, #25 and #26, Boundaries \
             (review plugin); story #33",
};

const PRECONDITIONS: HookMetadata = HookMetadata {
    event: HookEvent::PreToolUse,
    matcher: "Edit|Write|MultiEdit|NotebookEdit",
    fingerprint: "sha256:9df1ee0ab7205171",
    on_failure: OnFailure::Block,
    role: Role::Guard,
    model_generation: CLAUDE_5,
    source: "Blueprint: Hooks (preconditions), Feature catalog #4 and #20; story #23",
};

/// A guard although its entry fails open: its own failures must never block an edit batch.
const FAST_CHECK: HookMetadata = HookMetadata {
    event: HookEvent::PostToolBatch,
    matcher: "",
    fingerprint: "sha256:066536530af1eeaa",
    on_failure: OnFailure::Continue,
    role: Role::Guard,
    model_generation: CLAUDE_5,
    source: "Blueprint: Hooks (fast-check), check placement by cost; story #24",
};

const COMPLETENESS: HookMetadata = HookMetadata {
    event: HookEvent::Stop,
    matcher: "",
    fingerprint: "sha256:a79f91b8222e3cd6",
    on_failure: OnFailure::Continue,
    role: Role::Guard,
    model_generation: CLAUDE_5,
    source: "Blueprint: Hooks (completeness), Feature catalog #21 and #22, skill rule 16; \
             story #26",
};

const STATUS: HookMetadata = HookMetadata {
    event: HookEvent::SubagentStop,
    matcher: "",
    fingerprint: "sha256:d90a2774a4960bac",
    on_failure: OnFailure::Continue,
    role: Role::Scaffolding,
    model_generation: CLAUDE_5,
    source: "Blueprint: Hooks (status), Feature catalog #23, platform conformance #7; story #26",
};

const ROUTE: HookMetadata = HookMetadata {
    event: HookEvent::UserPromptSubmit,
    matcher: "",
    fingerprint: "sha256:2b9db1a36d2934d8",
    on_failure: OnFailure::Continue,
    role: Role::Scaffolding,
    model_generation: CLAUDE_5,
    source: "Blueprint: Clef decision points (D3 route); story #31",
};

const RULED_BEFORE: HookMetadata = HookMetadata {
    event: HookEvent::UserPromptSubmit,
    matcher: "",
    fingerprint: "sha256:eef2a74167c28b96",
    on_failure: OnFailure::Continue,
    role: Role::Scaffolding,
    model_generation: CLAUDE_5,
    source: "Blueprint: Clef decision points (D6 ruled-before); story #31",
};

const INJECTION_SCAN: HookMetadata = HookMetadata {
    event: HookEvent::PostToolBatch,
    matcher: "",
    fingerprint: "sha256:7b9a78954ee46111",
    on_failure: OnFailure::Continue,
    role: Role::Scaffolding,
    model_generation: CLAUDE_5,
    source: "Story #39 (GSD pattern: flag instruction-shaped text in fetched content)",
};

const CONTEXT_MONITOR: HookMetadata = HookMetadata {
    event: HookEvent::PostToolBatch,
    matcher: "",
    fingerprint: "sha256:c3acbc901216234f",
    on_failure: OnFailure::Continue,
    role: Role::Scaffolding,
    model_generation: CLAUDE_5,
    source: "Blueprint: Hooks (context-monitor), context survival; story #38",
};

const HANDOFF: HookMetadata = HookMetadata {
    event: HookEvent::PreCompact,
    matcher: "",
    fingerprint: "sha256:8a112bc19b313e45",
    on_failure: OnFailure::Continue,
    role: Role::Scaffolding,
    model_generation: CLAUDE_5,
    source: "Blueprint: Hooks (handoff), context survival; story #38",
};

const BRIEF: HookMetadata = HookMetadata {
    event: HookEvent::PreToolUse,
    matcher: "Agent",
    fingerprint: "sha256:7138cdbabc332ae7",
    on_failure: OnFailure::Block,
    role: Role::Guard,
    model_generation: CLAUDE_5,
    source: "Blueprint: Orchestration (no brief, no spawn; no nested spawns); story #36",
};

const STANDING_ORDERS: HookMetadata = HookMetadata {
    event: HookEvent::SubagentStart,
    matcher: "",
    fingerprint: "sha256:caa18fb3d04187c1",
    on_failure: OnFailure::Continue,
    role: Role::Scaffolding,
    model_generation: CLAUDE_5,
    source: "Blueprint: Orchestration (standing orders in every subagent); story #36",
};

#[cfg(test)]
mod tests {
    use super::{HookName, OnFailure, Role};

    #[test]
    fn a_row_blocks_on_failure_exactly_when_the_hook_is_a_guard() {
        for hook in HookName::ALL {
            assert_eq!(
                hook.metadata().on_failure == OnFailure::Block,
                hook.is_guard(),
                "{}: onFailure block and failing closed go together",
                hook.as_str()
            );
        }
        assert_eq!(HookName::FastCheck.metadata().role, Role::Guard);
    }
}
