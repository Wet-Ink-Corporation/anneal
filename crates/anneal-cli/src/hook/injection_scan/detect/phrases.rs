//! Word heuristics: overrides of earlier instructions, directives addressed to an AI, and HTML
//! comments that address one. The tables are in [`words`].

mod words;

use super::tokens::{Token, Tokens};
use super::{Hit, Kind, Scan};
use words::Element::{self, Any, End};
use words::{
    ADDRESS, AI_FOLLOWERS, COMMENT_VERBS, OVERRIDE_SCOPES, OVERRIDE_TARGETS, OVERRIDE_VERBS,
    OVERRIDES, STRONG, WEAK, YOU,
};

/// How many tokens either side of a weak phrase an address is looked for.
const ADDRESS_REACH: usize = 40;
/// Words read after an override verb for a scope and a target.
const OVERRIDE_REACH: usize = 5;

/// Adds the word heuristics' hits in `tokens` to `found`. A hit inside an HTML comment is a
/// [`Kind::HiddenComment`] on the comment's first line; a comment with no phrase in it still
/// hits when it names an AI, says `you` and holds a directive verb.
pub(super) fn scan(tokens: &Tokens, found: &mut Scan) {
    let words = tokens.tokens.as_slice();
    let near = near_address(words);
    let mut counted = vec![false; tokens.comment_lines.len()];
    for (at, token) in words.iter().enumerate() {
        let addressed = token.comment.is_some() || near.get(at).copied().unwrap_or(false);
        let Some(kind) = kind_at(words, at, addressed) else {
            continue;
        };
        match token.comment {
            Some(index) => {
                if let Some(seen) = counted.get_mut(index) {
                    *seen = true;
                }
                let line = tokens
                    .comment_lines
                    .get(index)
                    .copied()
                    .unwrap_or(token.line);
                found.add(Hit {
                    kind: Kind::HiddenComment,
                    line,
                });
            }
            None => found.add(Hit {
                kind,
                line: token.line,
            }),
        }
    }
    // A comment's tokens are contiguous, so each chunk is one comment or the text between two.
    for chunk in words.chunk_by(|a, b| a.comment == b.comment) {
        let Some(index) = chunk.first().and_then(|token| token.comment) else {
            continue;
        };
        if counted.get(index).copied().unwrap_or(true) || !addresses_ai(chunk) {
            continue;
        }
        if let Some(line) = tokens.comment_lines.get(index) {
            found.add(Hit {
                kind: Kind::HiddenComment,
                line: *line,
            });
        }
    }
}

/// Whether a comment's tokens name an AI, say `you` and hold a [`COMMENT_VERBS`] word.
fn addresses_ai(comment: &[Token]) -> bool {
    let has = |words: &[&str]| {
        comment
            .iter()
            .any(|token| words.contains(&token.text.as_str()))
    };
    let named = comment
        .iter()
        .enumerate()
        .any(|(at, token)| is_address(comment, at, token));
    named && has(&YOU) && has(&COMMENT_VERBS)
}

/// The kind of phrase starting at `at`, if any; a weak phrase counts only when `addressed`.
fn kind_at(words: &[Token], at: usize, addressed: bool) -> Option<Kind> {
    if overrides_at(words, at) || OVERRIDES.iter().any(|p| matches_at(words, at, p)) {
        Some(Kind::Override)
    } else if STRONG.iter().any(|p| matches_at(words, at, p))
        || (addressed && WEAK.iter().any(|p| matches_at(words, at, p)))
    {
        Some(Kind::Directive)
    } else {
        None
    }
}

/// Whether `pattern` matches the tokens from `at`.
fn matches_at(tokens: &[Token], at: usize, pattern: &[Element]) -> bool {
    let mut next = tokens.iter().skip(at);
    for element in pattern {
        let token = next.next();
        match element {
            Any(texts) => {
                if !token.is_some_and(|token| texts.contains(&token.text.as_str())) {
                    return false;
                }
            }
            End => {
                return token.is_none_or(|token| {
                    !token.word || AI_FOLLOWERS.contains(&token.text.as_str())
                });
            }
        }
    }
    true
}

/// Whether an override starts at `at`: a verb, then within [`OVERRIDE_REACH`] words of the same
/// sentence a scope word and after it a target (`ignore all previous instructions`).
fn overrides_at(tokens: &[Token], at: usize) -> bool {
    let Some(verb) = tokens.get(at) else {
        return false;
    };
    if !verb.word || !OVERRIDE_VERBS.contains(&verb.text.as_str()) {
        return false;
    }
    let mut scoped = false;
    let following = tokens
        .iter()
        .skip(at.saturating_add(1))
        .take(OVERRIDE_REACH);
    for token in following {
        if !token.word {
            if matches!(token.text.as_str(), "." | "!" | "?" | ";" | ":") {
                return false;
            }
            continue;
        }
        let text = token.text.as_str();
        if scoped && OVERRIDE_TARGETS.contains(&text) {
            return true;
        }
        scoped |= OVERRIDE_SCOPES.contains(&text);
    }
    false
}

/// Whether each token is within [`ADDRESS_REACH`] tokens of an address to an AI.
fn near_address(tokens: &[Token]) -> Vec<bool> {
    // prefix[i] counts the addresses among tokens[..i].
    let mut prefix = Vec::with_capacity(tokens.len().saturating_add(1));
    let mut count = 0_usize;
    prefix.push(count);
    for (at, token) in tokens.iter().enumerate() {
        if is_address(tokens, at, token) {
            count = count.saturating_add(1);
        }
        prefix.push(count);
    }
    (0..tokens.len())
        .map(|at| {
            let low = prefix
                .get(at.saturating_sub(ADDRESS_REACH))
                .copied()
                .unwrap_or(0);
            let high_at = at
                .saturating_add(ADDRESS_REACH)
                .saturating_add(1)
                .min(tokens.len());
            let high = prefix.get(high_at).copied().unwrap_or(count);
            high > low
        })
        .collect()
}

/// Whether the token at `at` addresses an AI: an [`ADDRESS`] word, or `language` before
/// `model(s)`.
fn is_address(tokens: &[Token], at: usize, token: &Token) -> bool {
    let text = token.text.as_str();
    ADDRESS.contains(&text)
        || (text == "language"
            && tokens
                .get(at.saturating_add(1))
                .is_some_and(|next| matches!(next.text.as_str(), "model" | "models")))
}
