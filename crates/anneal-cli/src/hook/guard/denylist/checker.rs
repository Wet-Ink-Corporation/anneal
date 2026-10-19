//! One check of a command: every simple command it runs, in order, with what it has set so
//! far (the bytes lexed, the directory `cd` moved to, the variables assigned). What the merge
//! gate's probe notes on the way is in [`probe`].

mod probe;

use super::config::{self, Cwd};
use super::feed::{Feed, own_feed};
use super::gh::PrMerge;
use super::variables::{self, Variables};
use super::wrappers::{self, Program, Run};
use super::{Context, MAX_DEPTH, MAX_LEXED_BYTES, MAX_LEXED_MIB, braces, git, readings, rm};
use crate::hook::guard::shell::{self, ParseError, SimpleCommand, Word};
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
    /// The first of `GH_REPO` and `GH_HOST` a word of the command sets or names.
    gh_env: Option<&'static str>,
    /// The first `cd`, `pushd` or `popd` the command ran, so far.
    moved: Option<&'static str>,
    /// The first way the command may run a call the probe cannot read, so far: a program
    /// known only at run time, or a function or alias definition. Noted by the probe only.
    unseen: Option<&'static str>,
    /// Whether the command calls gh with a first or second operand known only at run time
    /// (`gh "$@"`). Only the probe reads gh calls.
    unseen_gh_call: bool,
    /// Whether a word of the command, or a here-document or here-string it reads, holds
    /// `merge` in any case, after quote removal. Noted by the probe only.
    mentions_merge: bool,
    /// The strongest way a gh word or call of the command may hide a merge, so far. Noted by
    /// the probe only.
    gh_hides: probe::GhHides,
    /// How many gh calls the probe read in full in the simple command it is checking: run as
    /// the program itself or under a known wrapper, with no arguments appended at run time and
    /// not found by scanning an unknown program's arguments.
    gh_calls_here: usize,
    /// How deep the check is inside [`Checker::arguments`]: a gh call found there is not read
    /// in full, since the unknown program may pass it other arguments (`parallel gh ::: pr`).
    argument_depth: usize,
}

/// The most commands arguments are read as in one check (H-13): each read walks the rest of
/// the arguments and may read some of them again, so the total is capped, which also bounds
/// how deeply such reads nest.
const MAX_ARGUMENT_COMMANDS: usize = 32;

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
            unseen: None,
            unseen_gh_call: false,
            mentions_merge: false,
            gh_hides: probe::GhHides::Nothing,
            gh_calls_here: 0,
            argument_depth: 0,
        }
    }

    /// `check`'s denial, when this check applies the denylist's entries; `None` for the probe.
    fn entry(&self, check: impl FnOnce() -> Option<Denial>) -> Option<Denial> {
        match self.entries {
            Entries::Denylist => check(),
            Entries::PrMerge => None,
        }
    }

    /// Checks shell text.
    pub(super) fn text(&mut self, text: &str, scope: Scope<'_>) -> Option<Denial> {
        self.lexed_by(text, scope, shell::parse)
    }

    /// Checks text read as `parse` lexes it.
    fn lexed_by(
        &mut self,
        text: &str,
        scope: Scope<'_>,
        parse: fn(&str) -> Result<Vec<SimpleCommand>, ParseError>,
    ) -> Option<Denial> {
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
        let commands = match parse(text) {
            Ok(commands) => commands,
            Err(error) => {
                return Some(unreadable(format!("the command cannot be read: {error}")));
            }
        };
        commands.iter().find_map(|command| {
            if command.defines_function {
                self.note_unseen(probe::DEFINITION);
            }
            self.note_mentions(command);
            self.variables.record(command);
            let own = own_feed(command, &commands);
            let feed = own.as_ref().unwrap_or(scope.feed);
            self.command(command, Scope { feed, ..scope })
        })
    }

    fn command(&mut self, command: &SimpleCommand, scope: Scope<'_>) -> Option<Denial> {
        let writes = self.variables.substitute(&command.writes);
        let words = self.variables.substitute(&command.words);
        let outer = self.enter_command();
        let denial = writes
            .iter()
            .flat_map(readings::of)
            .find_map(|target| config::redirect(&target, &self.cwd))
            .or_else(|| self.words(&words, scope));
        self.leave_command(&words, outer);
        denial
    }

    /// Checks one simple command's words brace-expanded, as the shell expands them before any
    /// program, wrapper or shell reads its options (`bash {-c,'rm -rf /'}` runs
    /// `bash -c 'rm -rf /'`). Brace expansion reads quotes as bash does ([`braces`]), so the
    /// expanded words are the ones that run and one reading is enough. A program word that
    /// holds a group once the words as written are unwrapped (`{rm,-rf,/}`,
    /// `sudo {sh,-c,'rm -rf /'}`) is denied as unreadable, as it was before the expanded
    /// reading.
    fn words(&mut self, words: &[Word], scope: Scope<'_>) -> Option<Denial> {
        if let Some(denial) = self.note_gh_env(words) {
            return Some(denial);
        }
        if wrappers::defines(words) {
            self.note_unseen(probe::DEFINITION);
        }
        if !words.iter().any(braces::has_group) {
            return self.unwrapped(words, scope);
        }
        if let Run::Command {
            program: Program::Unknown(word),
            ..
        } = wrappers::unwrap(words)
            && braces::has_group(word)
        {
            return Some(unreadable(format!(
                "the program {} is built by brace expansion; write the command out",
                word.text
            )));
        }
        let expanded = match braces::expand(words) {
            Ok(expanded) => expanded,
            Err(denial) => return Some(denial),
        };
        self.unwrapped(&expanded, scope)
    }

    /// Checks one reading of a simple command's words: its wrappers removed, what it runs.
    fn unwrapped(&mut self, words: &[Word], scope: Scope<'_>) -> Option<Denial> {
        let scope = self.moved_by_wrapper(words, scope);
        self.run(wrappers::unwrap(words), scope)
    }

    /// Checks what a simple command runs once its wrappers are removed.
    fn run(&mut self, run: Run<'_>, scope: Scope<'_>) -> Option<Denial> {
        match run {
            Run::Nothing => None,
            Run::Either(runs) => runs.into_iter().find_map(|run| self.run(run, scope)),
            Run::Script(text) => self.text(&text, scope.nested()),
            Run::Split(text) => self.lexed_by(&text, scope.nested(), shell::parse_without_extglob),
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

    /// Applies the entries for `program` (all of them when the program is not known), to the
    /// arguments as written and, when a word holds an expansion, again in their emptied
    /// reading ([`readings::arguments`]). `cd` is followed once: the first reading already
    /// sends it anywhere, and following the second too would move it from anywhere to one
    /// directory (`cd "$d"/x*` to `/x*`), so relative writes after it would no longer be judged
    /// as possibly protected. The second reading reads the same calls again, so the gh calls and
    /// argument commands it counts are dropped (each is counted once; it still stops at the
    /// budget while it runs, and reads no more than the first), and so are the merges it finds
    /// when the first reading found one: it is the same call (`gh pr merge 1$x` is one merge,
    /// its pull request known only at run time), while a merge only the emptied reading shows
    /// is kept.
    fn program(&mut self, program: Program<'_>, args: &[Word], scope: Scope<'_>) -> Option<Denial> {
        let merges_before = self.merges.len();
        let denial = self.reading(program, args, scope);
        if denial.is_some() || matches!(program, Program::Named("cd" | "pushd")) {
            return denial;
        }
        let emptied = readings::arguments(args)?;
        let (counted, found) = (self.gh_calls_here, self.merges.len());
        let commands = self.argument_commands;
        let denial = self.reading(program, &emptied, scope);
        self.gh_calls_here = counted;
        self.argument_commands = commands;
        if found > merges_before {
            self.merges.truncate(found);
        }
        denial
    }

    /// Applies the entries for `program` to one reading of its arguments, brace-expanded: a
    /// no-op for the words [`Checker::words`] expanded, kept so that no reading the entries
    /// judge holds a group.
    fn reading(&mut self, program: Program<'_>, args: &[Word], scope: Scope<'_>) -> Option<Denial> {
        let expanded = match braces::expand(args) {
            Ok(expanded) => expanded,
            Err(denial) => return Some(denial),
        };
        let args = expanded.as_ref();
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
            Program::Unknown(word) => {
                self.note_unseen(probe::RUN_TIME_PROGRAM);
                self.unknown(word, args, scope)
            }
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

    /// Whether `word` names a program whose arguments are read as a command: one the guard
    /// checks, a shell or a wrapper, and for the probe also `gh`.
    fn names_checked(&self, word: &Word) -> bool {
        wrappers::names_checked(word)
            || (self.entries == Entries::PrMerge && wrappers::names_program(word, "gh"))
    }

    /// The arguments of a program the guard does not know, read as a command from each word
    /// that names a program it checks or unwraps: an unlisted wrapper (`caffeinate`,
    /// `runuser -u root --`, `chroot /`) runs them. At most [`MAX_ARGUMENT_COMMANDS`] are read
    /// in one check; past that it denies. The arguments come brace-expanded
    /// ([`Checker::words`]), and a name brace expansion built (`caffeinate {rm,-rf,/}`) reads
    /// as written, so `rm -rf /` is judged and `brew install {git,jq}` is allowed as
    /// `brew install git jq` is.
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
            self.argument_depth = self.argument_depth.saturating_add(1);
            let denial = self.words(command, scope);
            self.argument_depth = self.argument_depth.saturating_sub(1);
            if denial.is_some() {
                return denial;
            }
        }
        None
    }

    /// A program word that is not a literal name (never one that holds a brace group, which
    /// [`Checker::words`] denies): read as the words its literal parts split into
    /// (`rm${IFS}-rf`), its arguments checked against every entry and read as a command
    /// ([`Checker::arguments`]), and then denied when it is a command's output or a variable
    /// this command set to a value known only at run time.
    fn unknown(&mut self, word: &Word, args: &[Word], scope: Scope<'_>) -> Option<Denial> {
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
