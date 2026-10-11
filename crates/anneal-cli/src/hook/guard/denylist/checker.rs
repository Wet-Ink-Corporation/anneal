//! One check of a command: every simple command it runs, in order, with what it has set so
//! far (the bytes lexed, the directory `cd` moved to, the variables assigned).

use super::config::{self, Cwd};
use super::feed::{Feed, own_feed};
use super::variables::{self, Variables};
use super::wrappers::{self, Program, Run};
use super::{Context, MAX_DEPTH, MAX_LEXED_BYTES, MAX_LEXED_MIB, braces, git, rm};
use crate::hook::guard::shell::{self, SimpleCommand, Word};
use crate::hook::guard::{Denial, Rule};

/// Where a command is checked: what its standard input gets (when it does not redirect it),
/// how deeply nested it is, and whether operands are appended at run time.
#[derive(Debug, Clone, Copy)]
pub(super) struct Scope<'a> {
    pub(super) feed: &'a Feed<'a>,
    pub(super) depth: usize,
    pub(super) appended: bool,
}

impl Scope<'_> {
    /// One level deeper, reading the same input.
    fn nested(self) -> Self {
        Self {
            depth: self.depth.saturating_add(1),
            appended: false,
            ..self
        }
    }
}

/// The state of one check: the context, the bytes lexed so far, where `cd` went and the
/// variables set.
#[derive(Debug)]
pub(super) struct Checker<'c> {
    context: &'c Context,
    lexed: usize,
    cwd: Cwd,
    variables: Variables,
}

/// A denial for a command the guard cannot read.
fn unreadable(reason: String) -> Denial {
    Denial::new(Rule::Unparseable, reason)
}

impl<'c> Checker<'c> {
    pub(super) fn new(context: &'c Context) -> Self {
        Self {
            context,
            lexed: 0,
            cwd: Cwd::Start,
            variables: Variables::default(),
        }
    }

    /// Checks shell text.
    pub(super) fn text(&mut self, text: &str, scope: Scope<'_>) -> Option<Denial> {
        if scope.depth > MAX_DEPTH {
            return Some(unreadable(format!(
                "the command nests bash -c, eval or find -exec deeper than {MAX_DEPTH}"
            )));
        }
        self.lexed = self.lexed.saturating_add(text.len());
        if self.lexed > MAX_LEXED_BYTES {
            return Some(unreadable(format!(
                "the command re-reads more than {MAX_LEXED_MIB} MiB through bash -c or eval"
            )));
        }
        let commands = match shell::parse(text) {
            Ok(commands) => commands,
            Err(error) => {
                return Some(unreadable(format!("the command cannot be read: {error}")));
            }
        };
        commands.iter().find_map(|command| {
            self.variables.record(command);
            let own = own_feed(command, &commands);
            let feed = own.as_ref().unwrap_or(scope.feed);
            self.command(command, Scope { feed, ..scope })
        })
    }

    fn command(&mut self, command: &SimpleCommand, scope: Scope<'_>) -> Option<Denial> {
        let writes = self.variables.substitute(&command.writes);
        let words = self.variables.substitute(&command.words);
        writes
            .iter()
            .find_map(|target| config::redirect(target, &self.cwd))
            .or_else(|| self.words(&words, scope))
    }

    /// Checks one simple command's words.
    fn words(&mut self, words: &[Word], scope: Scope<'_>) -> Option<Denial> {
        match wrappers::unwrap(words) {
            Run::Nothing => None,
            Run::Script(text) => self.text(&text, scope.nested()),
            Run::StdinScript(shell) => self.stdin_script(shell, scope),
            Run::Unseen(shell) => Some(unreadable(format!(
                "{shell} runs a script from a process substitution, known only at run time; \
                 pass the script as text the guard can read, as in bash -c '...'"
            ))),
            Run::Exec(actions) => actions.into_iter().find_map(|action| {
                let appended = Scope {
                    appended: true,
                    ..scope.nested()
                };
                self.words(action, appended)
            }),
            Run::Command {
                program,
                args,
                appended,
            } => {
                let scope = Scope {
                    appended: scope.appended || appended,
                    ..scope
                };
                match scope.feed {
                    Feed::Static(input) if appended => {
                        // `xargs` appends what a static `echo` pipes into it; the new argument
                        // list owns its words. Quotes are dropped, as xargs drops them.
                        let input = input.replace(['\'', '"', '\\'], "");
                        let line: Vec<Word> = args
                            .iter()
                            .cloned()
                            .chain(input.split_whitespace().map(variables::literal))
                            .collect();
                        self.program(program, &line, scope)
                    }
                    _ => self.program(program, args, scope),
                }
            }
        }
    }

    /// Checks the script `shell` reads from its standard input.
    fn stdin_script(&mut self, shell: &str, scope: Scope<'_>) -> Option<Denial> {
        match scope.feed {
            Feed::Empty | Feed::File => None,
            Feed::Static(script) => {
                let empty = Feed::Empty;
                self.text(
                    script,
                    Scope {
                        feed: &empty,
                        ..scope.nested()
                    },
                )
            }
            Feed::Unknown => Some(unreadable(format!(
                "{shell} reads its script from standard input, which is known only at run \
                 time; pass the script as text the guard can read, as in bash -c '...'"
            ))),
        }
    }

    /// Applies the entries for `program` (all of them when the program is not known).
    fn program(&mut self, program: Program<'_>, args: &[Word], scope: Scope<'_>) -> Option<Denial> {
        let args = match braces::expand(args) {
            Ok(args) => args,
            Err(denial) => return Some(denial),
        };
        let args = args.as_ref();
        match program {
            Program::Named("git") => {
                let scripts = git::settings(args).scripts;
                scripts
                    .into_iter()
                    .find_map(|script| self.text(script, scope.nested()))
                    .or_else(|| git::check(args))
            }
            Program::Named("rm") => rm::check(args, self.context, scope.appended)
                .or_else(|| config::check("rm", args, &self.cwd)),
            Program::Named("cd" | "pushd") => {
                self.cwd.change(args);
                None
            }
            Program::Named(name) => config::check(name, args, &self.cwd),
            Program::Unknown(word) => self.unknown(word, args, scope),
        }
    }

    /// A program word that is not a literal name: denied when brace expansion builds it, read
    /// as the words its literal parts split into (`rm${IFS}-rf`), its arguments checked
    /// against every entry, and then denied when it is a command's output or a variable this
    /// command set to a value known only at run time.
    fn unknown(&mut self, word: &Word, args: &[Word], scope: Scope<'_>) -> Option<Denial> {
        if braces::has_group(word) {
            return Some(unreadable(format!(
                "the program {} is built by brace expansion; write the command out",
                word.text
            )));
        }
        let fields = variables::fields(word);
        if !fields.is_empty() {
            // The split words and the arguments form a new command line, which owns them.
            let line: Vec<Word> = fields.into_iter().chain(args.iter().cloned()).collect();
            if let Some(denial) = self.words(&line, scope) {
                return Some(denial);
            }
        }
        let by_entry = git::check(args)
            .or_else(|| rm::check(args, self.context, scope.appended))
            .or_else(|| config::check("cp", args, &self.cwd));
        if by_entry.is_some() {
            return by_entry;
        }
        if let Some(source) = variables::runtime_source(word) {
            return Some(unreadable(format!(
                "the program {} is {source}, known only at run time; write the command out",
                word.text
            )));
        }
        let name = self.variables.set_here(word)?;
        Some(unreadable(format!(
            "the program comes from ${name}, which this command sets to a value known only at \
             run time; write the command out"
        )))
    }
}
