//! One check of a command: every simple command it runs, in order, with what it has set so
//! far (the bytes lexed, the directory `cd` moved to, the variables assigned).

use super::config::{self, Cwd};
use super::feed::{Feed, own_feed};
use super::gh::{self, PrMerge};
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
    /// How a wrapper of this command (or of one it nests in) changed the working directory
    /// first: `env --chdir`, `sudo --chdir`, `find -execdir`. Noted by the probe only.
    pub(super) moved: Option<&'static str>,
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

/// What a check looks for in the commands it reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Entries {
    /// The denylist's entries (`git`, `rm`, `config`): the guard.
    Denylist,
    /// `gh pr merge` calls, collected rather than denied: the merge gate's probe. The
    /// denylist's entries are not applied; what the checker cannot read still denies.
    PrMerge,
}

/// The state of one check: the context, what it looks for, the bytes lexed so far, where `cd`
/// went, the variables set and the `gh pr merge` calls found.
#[derive(Debug)]
pub(super) struct Checker<'c> {
    context: &'c Context,
    entries: Entries,
    lexed: usize,
    /// How many times arguments have been read as a command ([`Checker::arguments`]).
    argument_commands: usize,
    cwd: Cwd,
    variables: Variables,
    merges: Vec<PrMerge>,
    /// The first of [`GH_TARGET_VARIABLES`] a word of the command sets or names.
    gh_env: Option<&'static str>,
    /// The first `cd`, `pushd` or `popd` the command ran, so far.
    moved: Option<&'static str>,
}

/// The variables that point gh at another repository or host; the probe notes a command that
/// sets or names one (`GH_REPO=o/x gh ...`, `env`, `export`), wherever it does.
const GH_TARGET_VARIABLES: [&str; 2] = ["GH_REPO", "GH_HOST"];

/// The most `gh pr merge` calls one command may make before the probe refuses it (H-13).
const MAX_MERGES: usize = 8;

/// The most commands arguments are read as in one check (H-13): each read walks the rest of
/// the arguments and may read some of them again, so the total is capped, which also bounds
/// how deeply such reads nest.
const MAX_ARGUMENT_COMMANDS: usize = 32;

/// The one of [`GH_TARGET_VARIABLES`] `text` sets or names: `NAME`, `NAME=...` or `NAME+=...`.
fn gh_target(text: &str) -> Option<&'static str> {
    GH_TARGET_VARIABLES.into_iter().find(|name| {
        text.strip_prefix(name).is_some_and(|after| {
            after.is_empty() || after.starts_with('=') || after.starts_with("+=")
        })
    })
}

/// A denial for a command the guard cannot read.
fn unreadable(reason: String) -> Denial {
    Denial::new(Rule::Unparseable, reason)
}

impl<'c> Checker<'c> {
    pub(super) fn new(context: &'c Context, entries: Entries) -> Self {
        Self {
            context,
            entries,
            lexed: 0,
            argument_commands: 0,
            cwd: Cwd::Start,
            variables: Variables::default(),
            merges: Vec::new(),
            gh_env: None,
            moved: None,
        }
    }

    /// The `gh pr merge` calls found, in order, each once, each noting a [`GH_TARGET_VARIABLES`]
    /// the command sets anywhere.
    pub(super) fn into_merges(self) -> Vec<PrMerge> {
        let env = self.gh_env;
        self.merges
            .into_iter()
            .map(|merge| PrMerge {
                env: env.map(str::to_owned),
                ..merge
            })
            .collect()
    }

    /// Notes the first word of `words` that sets or names one of [`GH_TARGET_VARIABLES`]
    /// (`GH_REPO`, `GH_REPO=...`, `GH_REPO+=...`), in the probe only. A word holding a brace
    /// group is read as each word it expands to (`export GH_RE{PO,X}=o/x` sets `GH_REPO`).
    ///
    /// # Errors
    /// The unreadable denial when one word expands to more than the brace cap.
    fn note_gh_env(&mut self, words: &[Word]) -> Option<Denial> {
        if self.entries != Entries::PrMerge || self.gh_env.is_some() {
            return None;
        }
        for word in words {
            let expanded = match braces::expand(std::slice::from_ref(word)) {
                Ok(expanded) => expanded,
                Err(denial) => return Some(denial),
            };
            self.gh_env = expanded.iter().find_map(|word| gh_target(&word.text));
            if self.gh_env.is_some() {
                break;
            }
        }
        None
    }

    /// `check`'s denial, when this check applies the denylist's entries; `None` for the probe.
    fn entry(&self, check: impl FnOnce() -> Option<Denial>) -> Option<Denial> {
        match self.entries {
            Entries::Denylist => check(),
            Entries::PrMerge => None,
        }
    }

    /// Records the `gh pr merge` that `gh <args>` runs in `scope`, if any, with how the command
    /// changed directory before it; past [`MAX_MERGES`] denies.
    fn gh(&mut self, args: &[Word], scope: Scope<'_>) -> Option<Denial> {
        let merge = PrMerge {
            moved: scope.moved.or(self.moved),
            ..gh::pr_merge(args)?
        };
        if !self.merges.contains(&merge) {
            self.merges.push(merge);
        }
        (self.merges.len() > MAX_MERGES).then(|| {
            unreadable(format!(
                "the command runs more than {MAX_MERGES} gh pr merge calls; merge one pull \
                 request at a time"
            ))
        })
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
        if let Some(denial) = self.note_gh_env(words) {
            return Some(denial);
        }
        let scope = self.moved_by_wrapper(words, scope);
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
            Program::Named("gh") if self.entries == Entries::PrMerge => self.gh(args, scope),
            Program::Named("git") => {
                let scripts = git::settings(args).scripts;
                scripts
                    .into_iter()
                    .find_map(|script| self.text(script, scope.nested()))
                    .or_else(|| self.entry(|| git::check(args)))
            }
            Program::Named("rm") => self.entry(|| {
                rm::check(args, self.context, scope.appended)
                    .or_else(|| config::check("rm", args, &self.cwd))
            }),
            Program::Named("cd") => self.change_directory("cd", Some(args)),
            Program::Named("pushd") => self.change_directory("pushd", Some(args)),
            Program::Named("popd") => self.change_directory("popd", None),
            Program::Named(name) => {
                self.entry(|| config::check(name, args, &self.cwd))
                    .or_else(|| {
                        wrappers::may_run_arguments(name)
                            .then(|| self.arguments(args, scope))
                            .flatten()
                    })
            }
            Program::Unknown(word) => self.unknown(word, args, scope),
        }
    }

    /// Follows `cd` or `pushd <args>`, or `popd` (`None`: back to a directory the guard never
    /// saw, so relative paths may start anywhere), and notes the first change for the probe.
    fn change_directory(&mut self, how: &'static str, args: Option<&[Word]>) -> Option<Denial> {
        match args {
            Some(args) => self.cwd.change(args),
            None => self.cwd = Cwd::Unknown,
        }
        self.moved.get_or_insert(how);
        None
    }

    /// `scope`, noting in the probe how a wrapper of `words` changes the working directory
    /// before its command runs ([`wrappers::chdir`]); an outer one is kept.
    fn moved_by_wrapper<'s>(&self, words: &[Word], scope: Scope<'s>) -> Scope<'s> {
        if self.entries != Entries::PrMerge || scope.moved.is_some() {
            return scope;
        }
        Scope {
            moved: wrappers::chdir(words),
            ..scope
        }
    }

    /// Whether `word` names a program whose arguments are read as a command: one the guard
    /// checks, a shell or a wrapper, and for the probe also `gh`.
    fn names_checked(&self, word: &Word) -> bool {
        wrappers::names_checked(word)
            || (self.entries == Entries::PrMerge && wrappers::names_program(word, "gh"))
    }

    /// The arguments of a program the guard does not know, read as a command from each word
    /// that names a program it checks or unwraps: an unlisted wrapper (`caffeinate`,
    /// `runuser -u root --`, `chroot /`) runs them. At most [`MAX_ARGUMENT_COMMANDS`] are read
    /// in one check; past that it denies.
    fn arguments(&mut self, args: &[Word], scope: Scope<'_>) -> Option<Denial> {
        for (at, word) in args.iter().enumerate() {
            if !self.names_checked(word) {
                continue;
            }
            self.argument_commands = self.argument_commands.saturating_add(1);
            if self.argument_commands > MAX_ARGUMENT_COMMANDS {
                return Some(unreadable(format!(
                    "arguments name more than {MAX_ARGUMENT_COMMANDS} commands the guard \
                     checks; run them one by one"
                )));
            }
            let Some(command) = args.get(at..) else {
                continue;
            };
            if let Some(denial) = self.words(command, scope) {
                return Some(denial);
            }
        }
        None
    }

    /// A program word that is not a literal name: denied when brace expansion builds it, read
    /// as the words its literal parts split into (`rm${IFS}-rf`), its arguments checked
    /// against every entry and read as a command ([`Checker::arguments`]), and then denied
    /// when it is a command's output or a variable this command set to a value known only at
    /// run time.
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
        let by_entry = self.entry(|| {
            git::check(args)
                .or_else(|| rm::check(args, self.context, scope.appended))
                .or_else(|| config::check("cp", args, &self.cwd))
        });
        if by_entry.is_some() {
            return by_entry;
        }
        if let Some(denial) = self.arguments(args, scope) {
            return Some(denial);
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
