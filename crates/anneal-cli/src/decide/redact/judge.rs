//! One whole shell word judged as bash reads it. The guard gives each word its text with
//! quotes removed and line continuations dropped (`hook::guard::redact::word`), so a rule never
//! sees `Bearer\` plus a newline, or `"X-Api-Key: "` cut at its quote. [`judge`] reads that
//! text with the word rules of [`super::rules`] and says whether it still holds a secret and
//! what it owes the word after it. Inside one word (a `bash -c` body) a `;`, `&`, `|` or
//! newline ends what the text before it owed, so nothing is owed past a separator; and an
//! assignment (`x="Bearer"`) owes nothing, since the word after it is a command.

use super::REDACTED;
use super::rules::{
    Scheme, Then, basic_credential, find_secret, is_assignment, looks_like_token, scheme,
    split_punctuation, url_userinfo,
};
use super::tools::{self, Tool};

/// Characters that end a command inside a word's text.
const SEPARATORS: [char; 4] = [';', '&', '|', '\n'];

/// What a word owes the word after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Owes {
    /// Nothing: the next word is read as usual.
    Nothing,
    /// The next word is a secret: a credential option's value, the word after `Bearer`.
    Word,
    /// The next word follows `Basic`: a secret unless [`basic_credential`] says it cannot be.
    BasicWord,
    /// The word named a header and gave it no value (`"X-Api-Key: "`): the value is the words
    /// after it.
    Header,
}

impl From<Scheme> for Owes {
    fn from(found: Scheme) -> Self {
        match found {
            Scheme::Bearer => Self::Word,
            Scheme::Basic => Self::BasicWord,
        }
    }
}

/// A word's text judged as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Judged {
    /// Whether some of the text is a secret that no [`REDACTED`] mark replaces yet.
    pub(crate) secret: bool,
    /// What the word owes the word after it.
    pub(crate) owes: Owes,
}

/// `text`, one word with its quotes removed, judged with the word rules; `tool` is the known
/// tool named before it, if any.
pub(crate) fn judge(text: &str, tool: Option<Tool>) -> Judged {
    let mut secret = false;
    let mut owes = Owes::Nothing;
    let mut tool = tool;
    // A line continuation inside the text (a `bash -c` body) is no character to bash.
    let joined = text.replace("\\\n", "");
    for segment in joined.split(SEPARATORS) {
        let mut reading = Reading::new(tool.take());
        for token in segment.split_whitespace() {
            reading.token(token);
        }
        secret |= reading.secret;
        owes = reading.owed();
    }
    if is_assignment(text) {
        owes = Owes::Nothing;
    }
    Judged { secret, owes }
}

/// The reading of one command's tokens within a word.
#[derive(Debug)]
struct Reading {
    tool: Option<Tool>,
    /// What the tokens so far owe the next token.
    owes: Owes,
    /// In a header's value: whether a token of it has been read.
    header: Option<bool>,
    /// Whether a token read is a secret not yet marked.
    secret: bool,
}

impl Reading {
    const fn new(tool: Option<Tool>) -> Self {
        Self {
            tool,
            owes: Owes::Nothing,
            header: None,
            secret: false,
        }
    }

    /// Reads one blank-separated token; one holding [`REDACTED`] is already hidden.
    fn token(&mut self, token: &str) {
        let hidden = token.contains(REDACTED);
        let (_, core, _) = split_punctuation(token);
        let owed = std::mem::replace(&mut self.owes, Owes::Nothing);
        let value = match (self.header.as_mut(), owed) {
            (Some(read), _) => {
                *read = true;
                true
            }
            (None, Owes::Word | Owes::Header) => true,
            (None, Owes::BasicWord) => basic_credential(token),
            (None, Owes::Nothing) => false,
        };
        if value {
            // A secret itself; a scheme word in a header's value owes its credential.
            self.secret |= !hidden;
            self.owes = scheme(core).map_or(Owes::Nothing, Owes::from);
            return;
        }
        if let Some(found) = find_secret(token, self.tool) {
            self.secret |= found.redacts && !hidden;
            self.owes = match found.then {
                Then::NextWord => Owes::Word,
                Then::AttachedScheme(attached) => Owes::from(attached),
                Then::Header => {
                    let rest = token.get(found.shown.len()..).unwrap_or_default();
                    let (_, value, _) = split_punctuation(rest);
                    self.header = Some(!value.is_empty());
                    scheme(value).map_or(Owes::Nothing, Owes::from)
                }
                Then::AttachedHeader | Then::Nothing => Owes::Nothing,
            };
        } else {
            self.secret |= !hidden && (looks_like_token(core) || url_userinfo(token).is_some());
            self.owes = scheme(core).map_or(Owes::Nothing, Owes::from);
        }
        if let Some(tool) = self.tool.as_mut()
            && tool.note(token)
        {
            self.owes = Owes::Word;
        }
        if let Some(tool) = tools::known(token) {
            self.tool = Some(tool);
        }
    }

    /// What the tokens read owe the next word: a header named with no value owes its value.
    fn owed(&self) -> Owes {
        match (self.header, self.owes) {
            (Some(false), Owes::Nothing) => Owes::Header,
            (_, owes) => owes,
        }
    }
}
