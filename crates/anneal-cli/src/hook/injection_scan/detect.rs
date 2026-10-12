//! The scanner: instruction-shaped text in content an agent read. Pure, no I/O.
//!
//! Every heuristic is a hand-written matcher over the text's words ([`tokens`], in
//! [`phrases`]) or its lines ([`lines`]); each runs in time linear in the text with a bounded
//! look-ahead. See `docs/injection-scan.md` for the heuristics and their known gaps.

mod lines;
mod phrases;
mod tokens;

/// What a hit looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Kind {
    /// An instruction to ignore or replace earlier instructions.
    Override,
    /// A directive addressed to an AI reading the content.
    Directive,
    /// An HTML comment, hidden when rendered, that directs an AI.
    HiddenComment,
    /// Chat-template or tool-call markup.
    Markup,
    /// JSON shaped like a tool call.
    ToolCallJson,
    /// Zero-width characters, Unicode tag characters or bidi controls.
    HiddenCharacters,
}

impl Kind {
    /// The kind, as the hook's note names it.
    pub(crate) const fn describe(self) -> &'static str {
        match self {
            Self::Override => "an instruction to ignore earlier instructions",
            Self::Directive => "a directive addressed to an AI",
            Self::HiddenComment => "a hidden HTML comment directing an AI",
            Self::Markup => "chat or tool-call markup",
            Self::ToolCallJson => "tool-call-shaped JSON",
            Self::HiddenCharacters => "zero-width, tag or bidi control characters",
        }
    }
}

/// One hit: its kind and the 1-based line of the scanned text it starts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Hit {
    /// What it looks like.
    pub(crate) kind: Kind,
    /// Its line, from 1.
    pub(crate) line: usize,
}

/// What a scan found: the first hit by line (then kind), and how many hits in all.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Scan {
    /// The hit on the lowest line.
    pub(crate) first: Option<Hit>,
    /// Every hit, the first included.
    pub(crate) count: usize,
}

impl Scan {
    /// Counts `hit`, keeping it as the first when it comes earlier.
    fn add(&mut self, hit: Hit) {
        self.count = self.count.saturating_add(1);
        let earlier = self
            .first
            .is_none_or(|first| (hit.line, hit.kind) < (first.line, first.kind));
        if earlier {
            self.first = Some(hit);
        }
    }
}

/// Scans `text` for instruction-shaped passages.
pub(crate) fn scan(text: &str) -> Scan {
    let mut found = Scan::default();
    phrases::scan(&tokens::tokenize(text), &mut found);
    lines::markup(text, &mut found);
    lines::hidden_characters(text, &mut found);
    found
}

#[cfg(test)]
mod tests;
