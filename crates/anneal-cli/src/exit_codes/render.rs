//! Renders the exit-code table for `--help` and for `docs/exit-codes.md`.

use std::fmt::{self, Display, Formatter};

use super::{Code, CommandCodes, GLOBAL};
use crate::command::CommandName;

/// Path of the generated document, relative to the repository root.
pub const DOC_PATH: &str = "docs/exit-codes.md";

/// `after_help` for `anneal --help`: the global codes, then one line per command.
#[must_use]
pub fn overview_help() -> String {
    format!(
        "Exit codes (all commands):\n{}\nExit codes per command:\n{}\n\
         See docs/exit-codes.md. Only success exits 0.",
        Rows(GLOBAL),
        Overview
    )
}

/// `after_help` for `anneal <command> --help`: its codes, the global ones, and its claim.
#[must_use]
pub fn command_help(command: CommandName) -> String {
    let table = CommandCodes::of(command);
    let claim = command.claim();
    format!(
        "Exit codes ({}):\n{}\nExit codes (all commands):\n{}\n\
         A green result proves: {}.\nIt does NOT prove: {}.",
        table.title(),
        Rows(table.codes),
        Rows(GLOBAL),
        claim.proves,
        claim.does_not_prove
    )
}

/// The full contents of `docs/exit-codes.md`.
#[must_use]
pub fn markdown() -> String {
    format!(
        "{MARKDOWN_PREAMBLE}## All commands\n\n{}{}",
        MarkdownTable(GLOBAL),
        MarkdownSections
    )
}

const MARKDOWN_PREAMBLE: &str = "\
<!-- Generated from crates/anneal-cli/src/exit_codes.rs. Do not edit by hand: run
     ANNEAL_BLESS=1 cargo test -p anneal-cli exit_codes_doc to regenerate. -->
# Exit codes

Every `anneal` command exits with a documented code. Codes mean different things for different
commands (`1` is \"cycle\" for `dag` and \"test red\" for `task`), so each command has its own
table. A few codes mean the same for every command. Only success exits 0: no failure path exits
0 (\"2 never reads as 0\"), and the JSON envelope's `ok` is true exactly when the code is 0.

Usage errors keep clap's 2 because Claude Code reads a hook's exit 2 as \"block\", so a
mis-invoked guard fails closed. A usage error writes nothing on stdout, while a command's own
non-zero code always comes with a JSON envelope on stdout, which tells the two apart. Every
error writes exactly one `ERROR <reason>` line on stderr.

There is no `anneal hook` command yet (#13). Until it exists, `hooks.json` must not call one: the
usage error would exit 2 and block the session.

";

/// One indented `code meaning` line per code.
struct Rows<'a>(&'a [Code]);

impl Display for Rows<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        for code in self.0 {
            writeln!(f, "  {:<3} {}", code.value(), code.meaning())?;
        }
        Ok(())
    }
}

/// One line per command: its title, then its codes joined by ` · `.
struct Overview;

impl Display for Overview {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let width = CommandName::ALL
            .iter()
            .map(|c| CommandCodes::of(*c).title().len())
            .max()
            .unwrap_or(0);
        for command in CommandName::ALL {
            let table = CommandCodes::of(command);
            write!(f, "  {:<width$} ", table.title())?;
            for (index, code) in table.codes.iter().enumerate() {
                let separator = if index == 0 { " " } else { " · " };
                write!(f, "{separator}{} {}", code.value(), code.meaning())?;
            }
            writeln!(f)?;
        }
        Ok(())
    }
}

/// A two-column markdown table of codes.
struct MarkdownTable<'a>(&'a [Code]);

impl Display for MarkdownTable<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("| Code | Meaning |\n|---|---|\n")?;
        for code in self.0 {
            writeln!(f, "| {} | {} |", code.value(), code.meaning())?;
        }
        Ok(())
    }
}

/// One markdown section per command: heading, table, and its claim.
struct MarkdownSections;

impl Display for MarkdownSections {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        for command in CommandName::ALL {
            let table = CommandCodes::of(command);
            let claim = command.claim();
            write!(
                f,
                "\n## `anneal {}`\n\n{}\nA green result proves {}. \
                 It does **not** prove {}.\n",
                table.title(),
                MarkdownTable(table.codes),
                claim.proves,
                claim.does_not_prove
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{DOC_PATH, command_help, markdown, overview_help};
    use crate::command::CommandName;
    use std::path::Path;

    #[test]
    fn exit_codes_doc_matches_the_table() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(DOC_PATH);
        let expected = markdown();
        if std::env::var_os("ANNEAL_BLESS").is_some() {
            std::fs::write(&path, &expected).expect("write docs/exit-codes.md");
        }
        let committed = std::fs::read_to_string(&path).expect("docs/exit-codes.md exists");
        assert!(
            committed == expected,
            "{DOC_PATH} drifted from the exit-code table; regenerate it with \
             `ANNEAL_BLESS=1 cargo test -p anneal-cli exit_codes_doc`"
        );
    }

    #[test]
    fn overview_lists_every_command_and_the_global_codes() {
        let help = overview_help();
        for command in CommandName::ALL {
            assert!(
                help.lines()
                    .any(|l| l.trim_start().starts_with(command.as_str())),
                "{command} has a line in the overview"
            );
        }
        assert!(
            help.contains("64  not implemented yet"),
            "global codes listed"
        );
    }

    #[test]
    fn command_help_shows_its_codes_and_what_it_does_not_prove() {
        let help = command_help(CommandName::Dag);
        assert!(
            help.starts_with(
                "Exit codes (dag check|frontier):\n  0   ok\n  1   cycle\n  2   dangling dependency\n"
            ),
            "got {help}"
        );
        assert!(
            help.ends_with("It does NOT prove: that the dependencies are the right ones."),
            "got {help}"
        );
    }
}
