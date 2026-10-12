//! Prose as Clef sees it: [`safe_prompt`].

use super::rules::{
    Scheme, Secret, Then, basic_credential, find_secret, is_pair, is_punctuation, looks_like_token,
    quote_after, scheme, split_punctuation, url_userinfo,
};
use super::tools::{self, Tool};
use super::{MAX_PROMPT_CHARS, REDACTED};
use crate::decide::state::cut_chars;

/// Words of prose redacted after a header name outside quotes (`Authorization: Bearer x`): its
/// scheme and credential.
const HEADER_WORDS: usize = 2;

/// `prompt` as it may be sent to Clef: see the module documentation of `decide::redact`.
pub(crate) fn safe_prompt(prompt: &str) -> String {
    let mut prose = Prose::default();
    let mut chars = 0_usize;
    let mut words = prompt.split_whitespace();
    while let Some(word) = words.next() {
        let before = prose.safe.len();
        if !prose.safe.is_empty() {
            prose.safe.push(' ');
        }
        let went_on = prose.word(word, &mut words);
        let pushed = prose
            .safe
            .get(before..)
            .map_or(0, |piece| piece.chars().count());
        chars = chars.saturating_add(pushed);
        // Past the cap the rest would be cut anyway; an unclosed quote took the rest.
        if !went_on || chars > MAX_PROMPT_CHARS {
            break;
        }
    }
    cut_chars(&prose.safe, MAX_PROMPT_CHARS)
}

/// The prose made safe so far, and what the words before say about the next ones.
#[derive(Debug, Default)]
struct Prose {
    safe: String,
    /// How many of the next words are secrets (after `Bearer`, `--password`, a header name).
    pending: usize,
    /// Words are secrets until this quote, opened with a header name, closes.
    header_quote: Option<char>,
    /// The word before was `Basic`.
    after_basic: bool,
    /// The last known tool named, whose secret options are redacted from then on.
    tool: Option<Tool>,
    /// Where the word before stands in an unquoted header's value.
    header: HeaderValue,
}

/// Where a word stands in the value of a header outside quotes, whose words [`Prose::pending`]
/// counts; a quoted one runs to its close instead ([`Prose::header_quote`]).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum HeaderValue {
    /// Not in a header's value.
    #[default]
    Outside,
    /// In a header's value.
    Inside,
    /// In a header's value, after a word ending in `;`: a `name=value` word next is another
    /// cookie pair ([`is_pair`]), so the value goes on; the first other word ends it.
    AfterSemicolon,
}

impl HeaderValue {
    /// Where a word of a header's value, `trail` its trailing punctuation, leaves the next.
    fn after(trail: &str) -> Self {
        if trail.contains(';') {
            Self::AfterSemicolon
        } else {
            Self::Inside
        }
    }
}

impl Prose {
    /// Writes `word` made safe. Returns `false` when it opened a quote that no later word
    /// closes, so the rest of the prompt went with it.
    fn word<'p>(&mut self, word: &'p str, words: &mut impl Iterator<Item = &'p str>) -> bool {
        let (lead, core, trail) = split_punctuation(word);
        let header = std::mem::take(&mut self.header);
        let pair = header == HeaderValue::AfterSemicolon && is_pair(word);
        if self.hides(core) || pair {
            if pair || (header != HeaderValue::Outside && self.pending > 0) {
                self.header = HeaderValue::after(trail);
            }
            self.safe.push_str(lead);
            self.safe.push_str(REDACTED);
            self.safe.push_str(trail);
            return self.after_hidden(word, words);
        }
        let went_on = self.write_secret(word, words).unwrap_or_else(|| {
            push_without_userinfo(word, &mut self.safe);
            true
        });
        self.note(core);
        went_on
    }

    /// Whether the word whose core is `core` is a secret as a whole.
    fn hides(&mut self, core: &str) -> bool {
        let after_basic = std::mem::take(&mut self.after_basic) && basic_credential(core);
        self.pending > 0 || self.header_quote.is_some() || after_basic || looks_like_token(core)
    }

    /// After a hidden word: inside a quoted header value, go on until its quote closes;
    /// otherwise a quote the word opens holds the rest of the secret (`-u "me:my pass"`), so the
    /// words through the one that closes it go too.
    fn after_hidden<'p>(&mut self, word: &str, words: &mut impl Iterator<Item = &'p str>) -> bool {
        self.pending = self.pending.saturating_sub(1);
        if let Some(open) = self.header_quote {
            self.header_quote = quote_after(Some(open), word);
            return true;
        }
        let open = opened_by(word);
        if open.is_some() {
            self.pending = 0;
        }
        skip_quoted(open, words)
    }

    /// The first secret in `word`, tried as written, then without the punctuation before it
    /// (quotes kept first, since they may belong to a name: `("API_KEY"=x)`, `(K[0]=v)`), with
    /// the punctuation trimmed.
    fn secret_in<'w>(&self, word: &'w str) -> Option<(&'w str, Secret<'w>)> {
        let unquoted = word.trim_start_matches(|c| is_punctuation(c) && !matches!(c, '\'' | '"'));
        let rest = word.trim_start_matches(is_punctuation);
        [word, unquoted, rest].into_iter().find_map(|candidate| {
            let before = word.get(..word.len().saturating_sub(candidate.len()))?;
            Some((before, find_secret(candidate, self.tool)?))
        })
    }

    /// Writes `word` with its secret redacted and reads on as the secret says; `None` when it
    /// holds none, else whether the prompt goes on (as [`Prose::word`]).
    fn write_secret<'p>(
        &mut self,
        word: &str,
        words: &mut impl Iterator<Item = &'p str>,
    ) -> Option<bool> {
        let (before, secret) = self.secret_in(word)?;
        self.safe.push_str(before);
        self.safe.push_str(&secret.shown);
        if secret.redacts {
            self.safe.push_str(REDACTED);
        }
        let open = quote_after(None, word);
        Some(match secret.then {
            Then::Header => {
                if open.is_some() {
                    self.header_quote = open;
                } else {
                    self.pending = HEADER_WORDS;
                    self.header = HeaderValue::after(split_punctuation(word).2);
                }
                true
            }
            // The value went with this word. Cookie pairs go on only after a semicolon that
            // ends it, and otherwise a quote it opens holds the rest of the value, as for a key.
            Then::AttachedHeader => {
                if split_punctuation(word).2.contains(';') {
                    self.header = HeaderValue::AfterSemicolon;
                    true
                } else {
                    skip_quoted(open, words)
                }
            }
            Then::NextWord => {
                self.pending = self.pending.max(1);
                true
            }
            Then::Nothing => skip_quoted(open, words),
        })
    }

    /// Notes what a readable word says about the next ones: a scheme, a known tool, or its
    /// subcommand (which may take a secret argument).
    fn note(&mut self, core: &str) {
        match scheme(core) {
            Some(Scheme::Bearer) => self.pending = self.pending.max(1),
            Some(Scheme::Basic) => self.after_basic = true,
            None => {}
        }
        if let Some(tool) = self.tool.as_mut()
            && tool.note(core)
        {
            self.pending = self.pending.max(1);
        }
        if let Some(tool) = tools::known(core) {
            self.tool = Some(tool);
        }
    }
}

/// The quote a hidden word leaves open: one its punctuation or core opens and its trailing
/// punctuation does not close. A quote only in the trailing punctuation (`abc"`) closes an
/// earlier one rather than opening.
fn opened_by(word: &str) -> Option<char> {
    let (lead, core, trail) = split_punctuation(word);
    let open = quote_after(quote_after(None, lead), core)?;
    quote_after(Some(open), trail)
}

/// Consumes from `words` the rest of a quoted value: while a quote is `open`, the words up to
/// and including the one that closes it. Returns `false` when no word closes it, so the rest
/// of the prompt went with it.
fn skip_quoted<'p>(mut open: Option<char>, words: &mut impl Iterator<Item = &'p str>) -> bool {
    while open.is_some() {
        let Some(word) = words.next() else {
            return false;
        };
        open = quote_after(open, word);
    }
    true
}

/// Pushes `word` with the userinfo of every URL in it redacted.
fn push_without_userinfo(word: &str, out: &mut String) {
    let mut rest = word;
    while let Some((start, end)) = url_userinfo(rest)
        && let (Some(head), Some(tail)) = (rest.get(..start), rest.get(end..))
    {
        out.push_str(head);
        out.push_str(REDACTED);
        rest = tail;
    }
    out.push_str(rest);
}
