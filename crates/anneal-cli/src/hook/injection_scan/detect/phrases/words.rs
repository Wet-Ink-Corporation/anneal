//! The phrase and word tables the word heuristics match, lowercase.

/// One element of a phrase.
#[derive(Debug, Clone, Copy)]
pub(super) enum Element {
    /// A token whose text is one of these.
    Any(&'static [&'static str]),
    /// The end of the phrase: the text ends, punctuation follows, or a word that keeps the
    /// address on the agent (`an AI agent`), so `an AI researcher` does not match.
    End,
}

use Element::{Any, End};

/// What a conditional address names.
const AI: &[&str] = &["ai", "llm", "chatbot"];
/// Words after an address that keep it on the agent.
pub(super) const AI_FOLLOWERS: [&str; 10] = [
    "assistant",
    "assistants",
    "agent",
    "agents",
    "model",
    "models",
    "system",
    "systems",
    "reading",
    "tool",
];

/// Phrases that address an AI on their own.
pub(super) const STRONG: &[&[Element]] = &[
    &[
        Any(&["if"]),
        Any(&["you"]),
        Any(&["are"]),
        Any(&["an", "a"]),
        Any(AI),
        End,
    ],
    &[
        Any(&["if"]),
        Any(&["you're"]),
        Any(&["an", "a"]),
        Any(AI),
        End,
    ],
    &[
        Any(&["if"]),
        Any(&["you"]),
        Any(&["are"]),
        Any(&["a"]),
        Any(&["language", "large"]),
        Any(&["language", "model"]),
    ],
    &[
        Any(&["if"]),
        Any(&["you're"]),
        Any(&["a"]),
        Any(&["language", "large"]),
        Any(&["language", "model"]),
    ],
    &[
        Any(&["ai", "llm"]),
        Any(&[
            "agents",
            "assistants",
            "models",
            "agent",
            "assistant",
            "model",
        ]),
        Any(&[
            "reading",
            "processing",
            "parsing",
            "summarizing",
            "summarising",
        ]),
        Any(&["this"]),
    ],
    &[Any(&["attention"]), Any(&["ai", "llm"]), End],
    &[Any(&["attention"]), Any(&[","]), Any(&["ai", "llm"]), End],
    &[
        Any(&["attention"]),
        Any(&["claude", "chatgpt", "assistant", "assistants"]),
    ],
];

/// Overrides without a target word (`disregard the above.`).
pub(super) const OVERRIDES: &[&[Element]] = &[
    &[
        Any(&["ignore", "disregard"]),
        Any(&["everything", "anything"]),
        Any(&["above", "before"]),
    ],
    &[
        Any(&["ignore", "disregard", "forget"]),
        Any(&["the"]),
        Any(&["above", "preceding"]),
        End,
    ],
];

/// The modes a jailbreak claims the agent is now in (`you are now in developer mode`); `you are
/// now in` alone is ordinary prose (`the branch you are now in`).
const MODES: &[&str] = &[
    "developer",
    "dan",
    "god",
    "admin",
    "unrestricted",
    "maintenance",
    "debug",
    "jailbreak",
];

/// Who a hidden directive is kept from.
const USER: &[&str] = &["user", "human", "operator"];

/// Phrases that are directives only near an address to an AI, or inside an HTML comment:
/// documentation says "you must now restart" to people.
pub(super) const WEAK: &[&[Element]] = &[
    &[Any(&["you"]), Any(&["must"]), Any(&["now"])],
    &[Any(&["from"]), Any(&["now"]), Any(&["on"])],
    &[
        Any(&["you"]),
        Any(&["are"]),
        Any(&["now"]),
        Any(&["a", "an", "my", "our", "the"]),
    ],
    &[
        Any(&["you're"]),
        Any(&["now"]),
        Any(&["a", "an", "my", "our", "the"]),
    ],
    &[
        Any(&["you"]),
        Any(&["are"]),
        Any(&["now"]),
        Any(&["in"]),
        Any(MODES),
        Any(&["mode"]),
    ],
    &[
        Any(&["you're"]),
        Any(&["now"]),
        Any(&["in"]),
        Any(MODES),
        Any(&["mode"]),
    ],
    &[
        Any(&["your"]),
        Any(&["new", "real", "actual", "true"]),
        Any(&[
            "task",
            "tasks",
            "instructions",
            "objective",
            "goal",
            "role",
            "purpose",
            "mission",
        ]),
    ],
    &[
        Any(&["do"]),
        Any(&["not"]),
        Any(&["tell", "inform", "alert", "notify"]),
        Any(&["the"]),
        Any(USER),
    ],
    &[
        Any(&["don't"]),
        Any(&["tell", "inform", "alert", "notify"]),
        Any(&["the"]),
        Any(USER),
    ],
    &[
        Any(&["without"]),
        Any(&["telling", "informing", "alerting", "notifying"]),
        Any(&["the"]),
        Any(USER),
    ],
    &[
        Any(&["hide", "conceal"]),
        Any(&["this", "it"]),
        Any(&["from"]),
        Any(&["the"]),
        Any(USER),
    ],
];

/// Words that address an AI (`language model` is matched as a pair).
pub(super) const ADDRESS: [&str; 11] = [
    "ai",
    "llm",
    "llms",
    "chatbot",
    "chatgpt",
    "claude",
    "copilot",
    "gemini",
    "gpt",
    "assistant",
    "assistants",
];

/// Words in a comment that make it a directive when it also names an AI and says `you`.
pub(super) const COMMENT_VERBS: [&str; 16] = [
    "must",
    "should",
    "ignore",
    "disregard",
    "forget",
    "execute",
    "run",
    "instead",
    "tell",
    "send",
    "reveal",
    "upload",
    "delete",
    "call",
    "fetch",
    "open",
];
/// Words that speak to the reader.
pub(super) const YOU: [&str; 3] = ["you", "your", "you're"];

/// Verbs that set earlier instructions aside.
pub(super) const OVERRIDE_VERBS: [&str; 6] = [
    "ignore",
    "disregard",
    "forget",
    "override",
    "bypass",
    "discard",
];
/// Words that point the verb at what came before or at the agent's own instructions.
pub(super) const OVERRIDE_SCOPES: [&str; 11] = [
    "previous",
    "prior",
    "above",
    "earlier",
    "preceding",
    "all",
    "your",
    "system",
    "original",
    "existing",
    "safety",
];
/// What is set aside.
pub(super) const OVERRIDE_TARGETS: [&str; 9] = [
    "instructions",
    "instruction",
    "prompt",
    "prompts",
    "directives",
    "directive",
    "guidelines",
    "guardrails",
    "programming",
];
