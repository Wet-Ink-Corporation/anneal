//! The JSON array a park pipes to `weighin batch`: one raise and its deferral, in one call, so
//! the item and its default land together (`@1` is the item the first operation raised).

use serde::Serialize;

/// A one-way call to park: what weigh-in is asked, and the default it falls back to.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ParkRequest<'a> {
    /// The question, one line.
    pub(crate) question: &'a str,
    /// What anneal did meanwhile, or `blocked`.
    pub(crate) did: &'a str,
    /// The fact that would flip the call.
    pub(crate) flip: &'a str,
    /// Where the call arises, `path:line`.
    pub(crate) at: &'a str,
    /// The ticket the call blocks.
    pub(crate) blocks: &'a str,
    /// When the default is taken, `YYYY-MM-DD`.
    pub(crate) resolve_by: &'a str,
    /// The option taken if the human does not decide.
    pub(crate) default: &'a str,
}

/// The door of every parked call: D2 parks only one-way doors.
const DOOR: &str = "one-way";

/// The authority of every parked call: it is the human's to make.
const AUTHORITY: &str = "authority";

/// The deferral's trigger.
const TRIGGER: &str = "next sitting";

/// The first operation's item, as weigh-in's batch names it.
const FIRST_ITEM: &str = "@1";

/// One operation of the batch.
#[derive(Debug, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Op<'a> {
    Raise {
        q: &'a str,
        door: &'a str,
        authority: &'a str,
        did: &'a str,
        flip: &'a str,
        at: &'a str,
        blocks: &'a str,
    },
    Defer {
        id: &'a str,
        flip: &'a str,
        trigger: &'a str,
        resolve_by: &'a str,
        default: &'a str,
    },
}

/// The batch for `request`: `[raise, defer]`.
///
/// # Errors
/// Only `serde_json`'s, which strings do not produce.
pub(crate) fn json(request: &ParkRequest<'_>) -> Result<String, serde_json::Error> {
    let ops = [
        Op::Raise {
            q: request.question,
            door: DOOR,
            authority: AUTHORITY,
            did: request.did,
            flip: request.flip,
            at: request.at,
            blocks: request.blocks,
        },
        Op::Defer {
            id: FIRST_ITEM,
            flip: request.flip,
            trigger: TRIGGER,
            resolve_by: request.resolve_by,
            default: request.default,
        },
    ];
    serde_json::to_string(&ops)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{ParkRequest, json as batch_json};

    #[test]
    fn a_park_is_one_raise_and_its_deferral_in_one_array() {
        let request = ParkRequest {
            question: "Should the cache be shared across tenants?",
            did: "blocked",
            flip: "a tenant needs isolation",
            at: "src/cache.rs:12",
            blocks: "7",
            resolve_by: "2026-10-19",
            default: "per-tenant",
        };

        let text = batch_json(&request).expect("encodes");

        let value: Value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(
            value,
            json!([
                {"op": "raise", "q": "Should the cache be shared across tenants?",
                 "door": "one-way", "authority": "authority", "did": "blocked",
                 "flip": "a tenant needs isolation", "at": "src/cache.rs:12", "blocks": "7"},
                {"op": "defer", "id": "@1", "flip": "a tenant needs isolation",
                 "trigger": "next sitting", "resolve_by": "2026-10-19", "default": "per-tenant"}
            ])
        );
        assert!(!text.contains('\n'), "one line: {text}");
    }
}
