//! Words whose value is set earlier in the same command: `F='rm -rf /'; $F` runs `rm -rf /`.
//!
//! A standalone assignment (`F=x`, also after `export`, `declare`, `local`, `typeset` or
//! `readonly`) with a literal value is followed, and its value put in where the variable is
//! used. `read`, `for`, `select`, `mapfile`, `printf -v`, `+=`, a value known only at run time,
//! or a second, different assignment leave the variable set but unknown. A program word that
//! uses such a variable, or is the output of a command substitution, is denied by the caller.

use std::borrow::Cow;
use std::collections::BTreeMap;

use super::wrappers::{is_assignment, is_option, skip_keywords};
use crate::hook::guard::shell::{Expansion, Shape, SimpleCommand, Word};

/// The variables the command has set so far, in the order it runs.
#[derive(Debug, Default)]
pub(super) struct Variables {
    /// Each one's value, when it is literal.
    set: BTreeMap<String, Option<String>>,
}

impl Variables {
    /// Records the variables `command` sets.
    pub(super) fn record(&mut self, command: &SimpleCommand) {
        let words = skip_keywords(&command.words);
        let Some(at) = words.iter().position(|word| !is_assignment(word)) else {
            for word in words {
                self.assign(word);
            }
            return;
        };
        // Assignments before a program only set its environment, not the shell's.
        let Some((_, [program, args @ ..])) = words.split_at_checked(at) else {
            return;
        };
        let name = if program.is_literal() {
            program.text.as_str()
        } else {
            ""
        };
        match name {
            "export" | "declare" | "typeset" | "local" | "readonly" => {
                for arg in args.iter().filter(|arg| is_assignment(arg)) {
                    self.assign(arg);
                }
            }
            "read" | "mapfile" | "readarray" => {
                for arg in args.iter().filter(|arg| !is_option(arg)) {
                    self.unknown(&arg.text);
                }
            }
            "for" | "select" => {
                if let Some(variable) = args.first() {
                    self.unknown(&variable.text);
                }
            }
            "printf" => {
                let target = args.windows(2).find_map(|pair| match pair {
                    [flag, variable] if flag.text == "-v" => Some(variable),
                    _ => None,
                });
                if let Some(variable) = target {
                    self.unknown(&variable.text);
                }
            }
            // `NAME+=value` appends to the variable.
            appended => {
                if let Some((variable, _)) = appended.split_once("+=").filter(|(n, _)| is_name(n)) {
                    self.unknown(variable);
                }
            }
        }
    }

    /// Records the assignment `word` (`NAME=value`).
    fn assign(&mut self, word: &Word) {
        let Some((name, value)) = word.text.split_once('=') else {
            return;
        };
        let value = word.is_literal().then(|| value.to_owned());
        match self.set.get_mut(name) {
            Some(old) if *old != value => *old = None,
            Some(_) => {}
            None => {
                self.set.insert(name.to_owned(), value);
            }
        }
    }

    /// Records `name` as set to a value known only at run time.
    fn unknown(&mut self, name: &str) {
        self.set.insert(name.to_owned(), None);
    }

    /// The first variable `word` uses that the command sets, if any.
    pub(super) fn set_here<'w>(&self, word: &'w Word) -> Option<&'w str> {
        word.expansions.iter().find_map(|expansion| {
            let raw = word.text.get(expansion.range.start..expansion.range.end)?;
            let name = referenced(raw)?;
            self.set.contains_key(name).then_some(name)
        })
    }

    /// `words` with each known value put in for its variable.
    pub(super) fn substitute<'w>(&self, words: &'w [Word]) -> Cow<'w, [Word]> {
        if self.set.is_empty() || !words.iter().any(|word| self.substituted(word).is_some()) {
            return Cow::Borrowed(words);
        }
        let mut line = Vec::with_capacity(words.len());
        for word in words {
            match self.substituted(word) {
                Some(replaced) => line.extend(replaced),
                // The new line owns its words: the unchanged ones are copied beside the new.
                None => line.push(word.clone()),
            }
        }
        Cow::Owned(line)
    }

    /// The words `word` becomes with known values put in, or `None` when it uses none, or when
    /// an unquoted value with spaces is only part of it (its splitting is not modelled).
    fn substituted(&self, word: &Word) -> Option<Vec<Word>> {
        let mut text = String::with_capacity(word.text.len());
        let mut remaining = Vec::new();
        let (mut replaced, mut splits) = (false, false);
        let mut at = 0;
        for expansion in &word.expansions {
            text.push_str(word.text.get(at..expansion.range.start)?);
            let raw = word.text.get(expansion.range.start..expansion.range.end)?;
            // After the command sets IFS, how an unquoted value splits is not modelled.
            let splits_unknown = !expansion.quoted && self.set.contains_key("IFS");
            let value = plain(raw)
                .and_then(|name| self.set.get(name)?.as_deref())
                .filter(|_| !splits_unknown);
            if let Some(value) = value {
                replaced = true;
                splits |= !expansion.quoted && value.contains(char::is_whitespace);
                text.push_str(value);
            } else {
                let start = text.len();
                text.push_str(raw);
                remaining.push(Expansion {
                    range: start..text.len(),
                    quoted: expansion.quoted,
                });
            }
            at = expansion.range.end;
        }
        text.push_str(word.text.get(at..)?);
        let whole =
            matches!(word.expansions.as_slice(), [only] if only.range == (0..word.text.len()));
        if !replaced || (splits && !whole) {
            return None;
        }
        if splits {
            return Some(text.split_whitespace().map(literal).collect());
        }
        let shape = if remaining.is_empty() {
            literal(&text).shape
        } else {
            Shape::Dynamic
        };
        Some(vec![Word {
            text,
            shape,
            quoted: word.quoted,
            expansions: remaining,
        }])
    }
}

/// `text` as a word with no expansion; a pattern when it holds a glob or brace character.
pub(super) fn literal(text: &str) -> Word {
    let pattern = text.contains(['*', '?', '[', '{']) || text.starts_with('~');
    Word {
        text: text.to_owned(),
        shape: if pattern {
            Shape::Pattern
        } else {
            Shape::Literal
        },
        quoted: false,
        expansions: Vec::new(),
    }
}

/// The variable a plain `$NAME` or `${NAME}` names.
fn plain(raw: &str) -> Option<&str> {
    let rest = raw.strip_prefix('$')?;
    let name = match rest.strip_prefix('{') {
        Some(braced) => braced.strip_suffix('}')?,
        None => rest,
    };
    is_name(name).then_some(name)
}

/// The variable an expansion's source text uses first (`$F`, `${F:-x}`), if any.
fn referenced(raw: &str) -> Option<&str> {
    let rest = raw.strip_prefix('$')?;
    let rest = rest.strip_prefix('{').unwrap_or(rest);
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    let name = rest.get(..end)?;
    is_name(name).then_some(name)
}

/// A shell identifier.
fn is_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Where `word`'s value comes from when it is nothing but expansions (and blanks) and one of
/// them is a command's output or the arguments of a function or script (`$@`, `$*`, `$1`):
/// values only known at run time, often a command line.
pub(super) fn runtime_source(word: &Word) -> Option<&'static str> {
    if !fields(word).is_empty() {
        return None;
    }
    word.expansions.iter().find_map(|expansion| {
        let raw = word.text.get(expansion.range.start..expansion.range.end)?;
        let parameter = raw.strip_prefix('$').map(|rest| {
            let rest = rest.strip_prefix('{').unwrap_or(rest);
            rest.trim_end_matches('}')
        });
        if (raw.starts_with("$(") && !raw.starts_with("$((")) || raw.starts_with('`') {
            Some("the output of a command substitution")
        } else if parameter.is_some_and(|name| {
            matches!(name, "@" | "*")
                || (!name.is_empty() && name.chars().all(|c| c.is_ascii_digit()))
        }) {
            Some("the arguments of the function or script")
        } else {
            None
        }
    })
}

/// The literal parts of `word` between its expansions, split at whitespace: what the word
/// becomes if each expansion is a separator, as `${IFS}` is. Empty when it has no expansion.
pub(super) fn fields(word: &Word) -> Vec<Word> {
    if word.expansions.is_empty() {
        return Vec::new();
    }
    let in_expansion = |at: usize| {
        word.expansions
            .iter()
            .any(|expansion| expansion.range.contains(&at))
    };
    let skeleton: String = word
        .text
        .char_indices()
        .map(|(at, c)| if in_expansion(at) { ' ' } else { c })
        .collect();
    skeleton.split_whitespace().map(literal).collect()
}
