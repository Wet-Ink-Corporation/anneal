//! Reading `weighin open --all --json` for the items weigh-in has decided: the rulings D6 can
//! offer before (or without) their decision atoms in `.kb/_intake/decisions/`.
//!
//! Only `id`, `status`, `question` and `decision.option` are read; every other key is ignored, so
//! a newer weigh-in that adds keys still reads. An item whose id is not a weigh-in id, or that
//! is not `decided`, is skipped.

use serde::Deserialize;

use super::ItemId;

/// The status of an item weigh-in has decided.
const DECIDED: &str = "decided";

/// An item as `open --all --json` lists it.
#[derive(Debug, Deserialize)]
struct ItemDto {
    id: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    question: String,
    #[serde(default)]
    decision: Option<DecisionDto>,
}

/// A decided item's decision.
#[derive(Debug, Deserialize)]
struct DecisionDto {
    option: String,
}

/// A ruling weigh-in holds: the item, its question and the option decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Decided {
    /// The item.
    pub(crate) id: ItemId,
    /// Its question.
    pub(crate) question: String,
    /// The option decided.
    pub(crate) option: String,
}

/// The decided items in `json`, in the order listed.
///
/// # Errors
/// `serde_json`'s when `json` is not an array of items.
pub(crate) fn decided(json: &[u8]) -> Result<Vec<Decided>, serde_json::Error> {
    let items: Vec<ItemDto> = serde_json::from_slice(json)?;
    Ok(items
        .into_iter()
        .filter(|item| item.status == DECIDED)
        .filter_map(|item| {
            let id = item.id.parse::<ItemId>().ok()?;
            let option = item.decision?.option;
            Some(Decided {
                id,
                question: item.question,
                option,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Decided, decided};

    #[test]
    fn only_decided_items_with_a_weigh_in_id_are_read() {
        let listed = json!([
            {"id": "wi-e41e02", "kind": "question", "status": "decided",
             "question": "Should the cache be shared across tenants?",
             "decision": {"option": "shared", "decider": "human:ryan", "note": "test"},
             "deferral": null, "records": []},
            {"id": "wi-b6ea09", "status": "deferred", "question": "Second q?", "decision": null},
            {"id": "wi-58067d", "status": "decided", "question": "No decision?"},
            {"id": "not-wi", "status": "decided", "question": "Q", "decision": {"option": "x"}}
        ]);

        let found = decided(listed.to_string().as_bytes()).expect("an array");

        assert_eq!(
            found,
            [Decided {
                id: "wi-e41e02".parse().expect("an id"),
                question: "Should the cache be shared across tenants?".to_owned(),
                option: "shared".to_owned(),
            }]
        );
    }

    #[test]
    fn an_empty_list_has_no_rulings_and_a_non_list_is_an_error() {
        assert_eq!(decided(b"[]").expect("an array"), []);
        let error = decided(b"{\"id\": \"wi-1\"}").expect_err("an object is not a list");
        assert_eq!(error.classify(), serde_json::error::Category::Data);
    }
}
