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

/// Whether `c` is a Unicode default-ignorable code point (`Default_Ignorable_Code_Point`,
/// Unicode 15.1): a renderer shows nothing for it. This is the one table the scan uses for
/// invisible characters: the word scanner drops them without ending a word, the
/// hidden-character check counts them, and a named place escapes them.
pub(crate) const fn is_ignorable(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{115F}'..='\u{1160}'
            | '\u{17B4}'..='\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF0}'..='\u{FFF8}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E0FFF}'
    )
}

/// Whether `c` prints nothing or breaks a line: a default-ignorable code point
/// ([`is_ignorable`]), or one of the other format, line separator and paragraph separator
/// characters (categories Cf, Zl and Zp, Unicode 15.1). A place holding one is shown escaped.
pub(crate) const fn is_unprintable(c: char) -> bool {
    is_ignorable(c)
        || matches!(
            c,
            '\u{0600}'..='\u{0605}'
                | '\u{06DD}'
                | '\u{070F}'
                | '\u{0890}'..='\u{0891}'
                | '\u{08E2}'
                | '\u{2028}'..='\u{2029}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{110BD}'
                | '\u{110CD}'
                | '\u{13430}'..='\u{1343F}'
        )
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
