//! What a command reading its standard input gets, so a shell reading its script from stdin
//! has the script checked: a static heredoc or here-string, or what `echo` or `printf` with
//! literal arguments pipes into it.

use std::borrow::Cow;

use super::wrappers::{self, Program, Run};
use crate::hook::guard::shell::{SimpleCommand, Stdin};

/// What a command reading its standard input gets.
#[derive(Debug)]
pub(super) enum Feed<'a> {
    /// Nothing: the Bash tool's own standard input.
    Empty,
    /// A file, which the guard does not read (like `bash script.sh`).
    File,
    /// Text known before the command runs.
    Static(Cow<'a, str>),
    /// Text known only at run time.
    Unknown,
}

/// What `command` reads when it redirects its standard input, or `None` when it inherits it.
pub(super) fn own_feed<'a>(
    command: &'a SimpleCommand,
    commands: &'a [SimpleCommand],
) -> Option<Feed<'a>> {
    Some(match &command.stdin {
        Stdin::Inherited => return None,
        Stdin::File => Feed::File,
        Stdin::Text(text) if text.is_literal() => Feed::Static(Cow::Borrowed(&text.text)),
        Stdin::Text(_) | Stdin::Pipe(None) => Feed::Unknown,
        Stdin::Pipe(Some(writer)) => commands
            .get(*writer)
            .and_then(printed)
            .map_or(Feed::Unknown, Feed::Static),
    })
}

/// The text `echo` or `printf` with literal arguments prints, when no escape or format
/// directive can change it.
fn printed(writer: &SimpleCommand) -> Option<Cow<'_, str>> {
    let Run::Command {
        program: Program::Named(name @ ("echo" | "printf")),
        args,
        ..
    } = wrappers::unwrap(&writer.words)
    else {
        return None;
    };
    let flags = if name == "echo" {
        args.iter()
            .take_while(|arg| matches!(arg.text.as_str(), "-n" | "-e" | "-E" | "-ne" | "-en"))
            .count()
    } else {
        0
    };
    let printed: Vec<&str> = args
        .iter()
        .skip(flags)
        .map(|arg| arg.is_literal().then_some(arg.text.as_str()))
        .collect::<Option<_>>()?;
    let text = printed.join(" ");
    let changes = text.contains('\\') || (name == "printf" && text.contains('%'));
    (!changes).then_some(Cow::Owned(text))
}
