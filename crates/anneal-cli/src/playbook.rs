//! `anneal playbook show <name>` and `anneal playbook list`: the playbooks built into the binary.
//!
//! The playbooks are the templates the `spec` and `plan` skills write from, the schemas of the
//! files the skills keep, the route playbook D3 chooses from and Clef's thresholds table. The
//! files live in `crates/anneal-cli/playbooks/`; each is included here and nowhere else, so the
//! command, `decide`, the route hook and the lints read the same bytes. `show` writes a
//! playbook's text in `data.content`, exactly as built in; an unknown name is a usage error
//! (exit 2, nothing on stdout). See `docs/playbook.md`.

use std::io::Write;

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::command::CommandName;
use crate::envelope::Envelope;
use crate::error::CliError;
use crate::exit_codes::{self, Code};

/// `anneal playbook`.
#[derive(Debug, Args)]
pub struct PlaybookArgs {
    #[command(subcommand)]
    action: PlaybookAction,
}

/// The `anneal playbook` subcommands.
#[derive(Debug, Subcommand)]
pub enum PlaybookAction {
    /// Write a built-in playbook's text, exactly as built in
    Show(ShowArgs),
    /// List the built-in playbooks
    List,
}

/// `anneal playbook show`.
#[derive(Debug, Args)]
pub struct ShowArgs {
    /// The playbook
    #[arg(value_enum, value_name = "NAME")]
    name: Name,
}

/// A built-in playbook. The set is closed: a new playbook is a new variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Name {
    /// `spec.template.md`: the spec template `anneal lint spec` checks against.
    SpecTemplate,
    /// `plan.template.md`: the plan template `anneal lint plan` checks against.
    PlanTemplate,
    /// `schemas/design-tree.schema.json`: the `grill` skill's design tree.
    DesignTreeSchema,
    /// `schemas/plan.schema.json`: a plan as `anneal lint plan` reads it (`data.plan`).
    PlanSchema,
    /// `route.json`: the phases and sizes Clef's D3 chooses from.
    Route,
    /// `clef-thresholds.json`: each decision point's model, fallback and bands.
    ClefThresholds,
}

/// How a playbook's text is to be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum MediaType {
    /// Markdown.
    #[serde(rename = "text/markdown")]
    Markdown,
    /// JSON.
    #[serde(rename = "application/json")]
    Json,
}

impl Name {
    /// Every playbook, in the order `playbook list` gives them.
    pub const ALL: [Self; 6] = [
        Self::SpecTemplate,
        Self::PlanTemplate,
        Self::DesignTreeSchema,
        Self::PlanSchema,
        Self::Route,
        Self::ClefThresholds,
    ];

    /// The playbook's text, exactly as built in.
    #[must_use]
    pub const fn content(self) -> &'static str {
        match self {
            Self::SpecTemplate => include_str!("../playbooks/spec.template.md"),
            Self::PlanTemplate => include_str!("../playbooks/plan.template.md"),
            Self::DesignTreeSchema => include_str!("../playbooks/schemas/design-tree.schema.json"),
            Self::PlanSchema => include_str!("../playbooks/schemas/plan.schema.json"),
            Self::Route => include_str!("../playbooks/route.json"),
            Self::ClefThresholds => include_str!("../playbooks/clef-thresholds.json"),
        }
    }

    /// How the playbook's text is to be read.
    #[must_use]
    pub const fn media_type(self) -> MediaType {
        match self {
            Self::SpecTemplate | Self::PlanTemplate => MediaType::Markdown,
            Self::DesignTreeSchema | Self::PlanSchema | Self::Route | Self::ClefThresholds => {
                MediaType::Json
            }
        }
    }

    /// One line on what the playbook is for, for `playbook list`.
    #[must_use]
    pub const fn summary(self) -> &'static str {
        match self {
            Self::SpecTemplate => "the spec template; a spec written from it passes lint spec",
            Self::PlanTemplate => "the plan template; a plan written from it passes lint plan",
            Self::DesignTreeSchema => "the JSON Schema of the grill skill's design tree",
            Self::PlanSchema => "the JSON Schema of a plan as lint plan reads it (data.plan)",
            Self::Route => "the phases and sizes Clef's D3 chooses from, with their skills",
            Self::ClefThresholds => {
                "each Clef decision point's model, fallback and bands; decide and calibrate \
                 use it unless --thresholds names a copy"
            }
        }
    }
}

/// `data` of `playbook show`.
#[derive(Debug, Serialize)]
struct Shown {
    name: Name,
    media_type: MediaType,
    content: &'static str,
}

/// One row of `playbook list`'s `data.playbooks`.
#[derive(Debug, Serialize)]
struct Listed {
    name: Name,
    media_type: MediaType,
    summary: &'static str,
}

/// `data` of `playbook list`.
#[derive(Debug, Serialize)]
struct Listing {
    playbooks: Vec<Listed>,
}

/// Runs `anneal playbook`, writing its envelope to `out`.
///
/// # Errors
/// [`CliError::Output`] when `out` cannot be written.
pub fn run<O: Write>(args: &PlaybookArgs, out: &mut O) -> Result<Code, CliError> {
    let emitted = match &args.action {
        PlaybookAction::Show(ShowArgs { name }) => {
            let data = Shown {
                name: *name,
                media_type: name.media_type(),
                content: name.content(),
            };
            Envelope::new(CommandName::Playbook, Some("show"), exit_codes::OK, data).emit(out)
        }
        PlaybookAction::List => {
            let playbooks = Name::ALL
                .into_iter()
                .map(|name| Listed {
                    name,
                    media_type: name.media_type(),
                    summary: name.summary(),
                })
                .collect();
            let data = Listing { playbooks };
            Envelope::new(CommandName::Playbook, Some("list"), exit_codes::OK, data).emit(out)
        }
    };
    emitted.map_err(CliError::Output)
}

#[cfg(test)]
mod tests {
    use super::{MediaType, Name};

    #[test]
    fn markdown_playbooks_are_the_templates_and_the_rest_are_json_objects() {
        for name in Name::ALL {
            match name.media_type() {
                MediaType::Markdown => assert!(
                    matches!(name, Name::SpecTemplate | Name::PlanTemplate),
                    "{name:?} is markdown"
                ),
                MediaType::Json => {
                    let parsed = serde_json::from_str::<serde_json::Value>(name.content());
                    assert!(
                        matches!(parsed, Ok(serde_json::Value::Object(_))),
                        "{name:?} is one JSON object: {parsed:?}"
                    );
                }
            }
        }
    }
}
