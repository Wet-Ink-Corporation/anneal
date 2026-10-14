//! Reading `weighin batch`'s receipt: the item a park minted (or found already raised) and its
//! deferral.
//!
//! weigh-in prints one line per operation. For a park's raise and defer (weigh-in 0.6):
//!
//! ```text
//! ✓ raised wi-e41e02 (raised)
//! ✓ deferred wi-e41e02 until next sitting (default per-tenant by 2026-10-19) · tracking atom …
//! ```
//!
//! and on a retry the first line is `✓ wi-e41e02 already raised`. A line is read by its first
//! words after the mark, never by searching it, since the question or the default can hold any
//! word. The park took when a raise line and a deferral line name the same item.

use super::ItemId;

/// What a receipt does not show.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ReceiptError {
    /// No line says an item was raised.
    #[error("the receipt names no raised wi- item")]
    NoRaise,
    /// The raised item has no deferral line.
    #[error("the receipt shows no deferral of {item}")]
    NoDeferral {
        /// The item that was raised.
        item: ItemId,
    },
}

/// One receipt line, as far as a park cares.
enum Line<'a> {
    /// `raised <id>` or `<id> already raised`.
    Raised(&'a str),
    /// `deferred <id> ...`.
    Deferred(&'a str),
    /// Anything else.
    Other,
}

/// Classifies `line` by its first words, after any mark (`✓`, `✗`) that starts it.
fn classify(line: &str) -> Line<'_> {
    let words: Vec<&str> = line
        .split_whitespace()
        .skip_while(|word| !word.starts_with(|c: char| c.is_ascii_alphanumeric()))
        .take(3)
        .collect();
    match words.as_slice() {
        ["raised", id, ..] | [id, "already", "raised"] => Line::Raised(id),
        ["deferred", id, ..] => Line::Deferred(id),
        _ => Line::Other,
    }
}

/// The item a park's receipt `text` raised and deferred.
///
/// # Errors
/// [`ReceiptError::NoRaise`] when no line raised a valid `wi-` id, and
/// [`ReceiptError::NoDeferral`] when the first one raised was not deferred.
pub(crate) fn parked_item(text: &str) -> Result<ItemId, ReceiptError> {
    let lines: Vec<Line<'_>> = text.lines().map(classify).collect();
    let raised = lines
        .iter()
        .find_map(|line| match line {
            Line::Raised(id) => id.parse::<ItemId>().ok(),
            Line::Deferred(_) | Line::Other => None,
        })
        .ok_or(ReceiptError::NoRaise)?;
    let deferred = lines
        .iter()
        .any(|line| matches!(line, Line::Deferred(id) if *id == raised.as_str()));
    if deferred {
        Ok(raised)
    } else {
        Err(ReceiptError::NoDeferral { item: raised })
    }
}

#[cfg(test)]
mod tests {
    use super::{ReceiptError, parked_item};

    /// weigh-in 0.6.1's receipt for a first park, verbatim.
    const FIRST: &str = "✓ raised wi-e41e02 (raised)\n\
        ✓ deferred wi-e41e02 until next sitting (default per-tenant by 2026-10-19) · tracking \
        atom .kb/_intake/open_questions/wi-e41e02-should-the-cache-be-shared-across-tenants.md\n";

    /// The same park run again: weigh-in finds the raise and defers again.
    const AGAIN: &str = "✓ wi-e41e02 already raised\n\
        ✓ deferred wi-e41e02 until next sitting (default per-tenant by 2026-10-19) · tracking \
        atom .kb/_intake/open_questions/wi-e41e02-should-the-cache-be-shared-across-tenants.md\n";

    #[test]
    fn a_first_park_names_the_minted_item() {
        let item = parked_item(FIRST).expect("raised and deferred");

        assert_eq!(item.as_str(), "wi-e41e02");
    }

    #[test]
    fn a_retried_park_names_the_item_already_raised() {
        let item = parked_item(AGAIN).expect("already raised, deferred again");

        assert_eq!(item.as_str(), "wi-e41e02");
    }

    #[test]
    fn a_receipt_without_a_raise_is_refused() {
        let receipts = [
            "",
            "weighin: EOF while parsing an object at line 2 column 0\n",
            "✓ deferred wi-e41e02 until next sitting\n",
            "✓ raised wi-NOT-AN-ID (raised)\n✓ deferred wi-NOT-AN-ID until next sitting\n",
        ];

        for receipt in receipts {
            assert_eq!(
                parked_item(receipt),
                Err(ReceiptError::NoRaise),
                "{receipt:?}"
            );
        }
    }

    #[test]
    fn a_raise_without_its_deferral_is_refused() {
        let receipt = "✓ raised wi-bc9073 (raised)\n✗ op 1: missing field `default`\n\
                       ✓ deferred wi-000000 until next sitting\n";

        let error = parked_item(receipt).expect_err("the deferral failed");

        assert_eq!(
            error.to_string(),
            "the receipt shows no deferral of wi-bc9073"
        );
    }

    #[test]
    fn a_question_that_says_raised_is_not_a_raise_line() {
        let receipt = "✓ deferred wi-aaaaaa until next sitting (default raised wi-bbbbbb)\n";

        assert_eq!(parked_item(receipt), Err(ReceiptError::NoRaise));
    }
}
