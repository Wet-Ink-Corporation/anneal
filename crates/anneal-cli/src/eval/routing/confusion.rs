//! The confusion table: rows are the expected skills, columns the skills the router picked
//! (Tier 1: the top-ranked skill; Tier 2: the skill that fired).

use std::collections::BTreeMap;

use serde::Serialize;

use crate::eval::routing::{Case, NO_MATCH, TIE};

/// The row-header cell of Tier 1's rendered table.
const CORNER: &str = "expected \\ top";

/// Counts of cases by expected skill (row) and top-ranked skill (column).
///
/// A positive passes only on the diagonal; a near miss passes when its owner outscores the
/// skill it mimics, so it can pass off the diagonal when a third skill ranks first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Confusion<'a> {
    /// `rows[expected][top label]`; every row has every column, zeros included.
    rows: BTreeMap<&'a str, BTreeMap<&'a str, usize>>,
    /// Column labels in display order: every skill, then `(tie)` and `(none)` when they occur.
    #[serde(skip)]
    columns: Vec<&'a str>,
    /// The row-header cell of the rendered table.
    #[serde(skip)]
    corner: &'static str,
}

impl<'a> Confusion<'a> {
    /// The table for `cases` over the model-invoked `skills` (sorted).
    pub(crate) fn new(skills: &'a [String], cases: &'a [Case]) -> Self {
        let pairs = cases
            .iter()
            .map(|case| (case.expected.as_str(), case.top.label()))
            .collect();
        Self::tally(CORNER, skills, &[TIE, NO_MATCH], pairs)
    }

    /// The table of `(expected, picked)` pairs over the model-invoked `skills` (sorted). The
    /// columns are every skill, then each of `extras` that some pair picked, in that order;
    /// `corner` heads the rendered rows.
    pub(crate) fn tally(
        corner: &'static str,
        skills: &'a [String],
        extras: &[&'a str],
        pairs: Vec<(&'a str, &'a str)>,
    ) -> Self {
        let mut columns: Vec<&str> = skills.iter().map(String::as_str).collect();
        for &extra in extras {
            if pairs.iter().any(|&(_, picked)| picked == extra) {
                columns.push(extra);
            }
        }
        let mut rows: BTreeMap<&str, BTreeMap<&str, usize>> = BTreeMap::new();
        for (expected, picked) in pairs {
            let row = rows
                .entry(expected)
                .or_insert_with(|| columns.iter().map(|&c| (c, 0)).collect());
            let count = row.entry(picked).or_insert(0);
            *count = count.saturating_add(1);
        }
        Self {
            rows,
            columns,
            corner,
        }
    }

    /// The table as aligned text lines: a header, then one line per expected skill.
    #[must_use]
    pub fn render(&self) -> Vec<String> {
        let label_width = self
            .rows
            .keys()
            .map(|name| name.chars().count())
            .chain([self.corner.chars().count()])
            .max()
            .unwrap_or(0);
        let widths: Vec<usize> = self
            .columns
            .iter()
            .map(|&column| {
                self.rows
                    .values()
                    .filter_map(|row| row.get(column))
                    .map(|count| count.to_string().len())
                    .chain([column.chars().count()])
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let line = |label: &str, cells: Vec<String>| {
            let mut parts = Vec::with_capacity(cells.len().saturating_add(1));
            parts.push(format!("{label:<label_width$}"));
            parts.extend(cells);
            parts.join("  ")
        };
        let mut lines = Vec::with_capacity(self.rows.len().saturating_add(1));
        let header = self.columns.iter().zip(&widths);
        lines.push(line(
            self.corner,
            header
                .map(|(column, width)| format!("{column:>width$}"))
                .collect(),
        ));
        for (name, row) in &self.rows {
            let cells = self.columns.iter().zip(&widths).map(|(column, width)| {
                let count = row.get(column).copied().unwrap_or(0);
                format!("{count:>width$}")
            });
            lines.push(line(name, cells.collect()));
        }
        lines
    }
}
