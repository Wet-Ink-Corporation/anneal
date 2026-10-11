//! The committed JSON Schema for design trees
//! (`plugin/playbooks/schemas/design-tree.schema.json`) agrees with the serde DTO that
//! `anneal grill` reads: the same fields, required fields, types, statuses and bounds.

use std::collections::BTreeSet;
use std::error::Error;
use std::path::Path;

use anneal_cli::grill::tree::{
    FORMAT_VERSION, MAX_DEPENDENCIES, MAX_NODES, MAX_TURNS, Node, Status, TreeFile, Turn,
};
use serde_json::Value;

type TestResult = Result<(), Box<dyn Error>>;

/// The id pattern `anneal grill` checks (`grill::tree` has the hand-written matcher).
const ID_PATTERN: &str = "^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$";
/// "Not blank": the pattern the schema gives the texts that must not be blank.
const NOT_BLANK: &str = "\\S";

fn schema() -> Result<Value, Box<dyn Error>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../plugin/playbooks/schemas/design-tree.schema.json");
    Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
}

/// A tree with one turn and one node, every field set, to read the DTO's field names from.
fn sample() -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::to_value(TreeFile {
        version: FORMAT_VERSION,
        goal: "export orders".to_owned(),
        turns: vec![Turn { confidence: 0.5 }],
        nodes: vec![Node {
            id: "format".to_owned(),
            question: "Which format?".to_owned(),
            depends_on: vec!["scope".to_owned()],
            status: Status::Open,
            answer: "CSV".to_owned(),
            is_decision: 0.75,
            confidence: 0.5,
        }],
    })?)
}

fn keys(value: Option<&Value>) -> BTreeSet<String> {
    value
        .and_then(Value::as_object)
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default()
}

fn strings(schema: &Value, pointer: &str) -> BTreeSet<String> {
    schema
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The JSON Schema `type` of a serialized value.
fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_u64() || n.is_i64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Each object level of the sample and where the schema describes it.
fn levels(sample: &Value) -> Result<[(&Value, &'static str); 3], Box<dyn Error>> {
    Ok([
        (sample, ""),
        (sample.pointer("/turns/0").ok_or("a turn")?, "/$defs/turn"),
        (sample.pointer("/nodes/0").ok_or("a node")?, "/$defs/node"),
    ])
}

#[test]
fn the_schema_fields_agree_with_the_design_tree_dto() -> TestResult {
    let schema = schema()?;
    let sample = sample()?;

    for (value, at) in levels(&sample)? {
        let fields = keys(Some(value));
        assert_eq!(
            strings(&schema, &format!("{at}/required")),
            fields,
            "{at}: every DTO field is required"
        );
        assert_eq!(
            keys(schema.pointer(&format!("{at}/properties"))),
            fields,
            "{at}: the properties are the DTO's fields"
        );
        assert_eq!(
            schema.pointer(&format!("{at}/additionalProperties")),
            Some(&Value::Bool(false)),
            "{at}: no other field, as the DTO denies unknown fields"
        );
    }
    Ok(())
}

#[test]
fn the_schema_property_types_agree_with_the_design_tree_dto() -> TestResult {
    let schema = schema()?;
    let sample = sample()?;

    for (value, at) in levels(&sample)? {
        for (field, field_value) in value.as_object().ok_or("an object")? {
            let pointer = format!("{at}/properties/{field}/type");
            assert_eq!(
                schema.pointer(&pointer),
                Some(&Value::from(json_type(field_value))),
                "{pointer} matches the DTO's {field}"
            );
        }
    }
    Ok(())
}

#[test]
fn the_schema_statuses_agree_with_the_design_tree_dto() -> TestResult {
    let expected: BTreeSet<String> = Status::ALL
        .iter()
        .map(|status| match serde_json::to_value(status)? {
            Value::String(name) => Ok(name),
            other => Err(format!("{other} is not a string").into()),
        })
        .collect::<Result<_, Box<dyn Error>>>()?;

    assert_eq!(
        strings(&schema()?, "/$defs/node/properties/status/enum"),
        expected,
        "status values"
    );
    Ok(())
}

#[test]
fn the_schema_bounds_agree_with_the_design_tree_checks() -> TestResult {
    let schema = schema()?;
    let expected = [
        ("/properties/version/const", Value::from(FORMAT_VERSION)),
        ("/properties/turns/maxItems", Value::from(MAX_TURNS)),
        ("/properties/nodes/maxItems", Value::from(MAX_NODES)),
        ("/$defs/id/pattern", Value::from(ID_PATTERN)),
        ("/$defs/probability/minimum", Value::from(0)),
        ("/$defs/probability/maximum", Value::from(1)),
        ("/properties/goal/pattern", Value::from(NOT_BLANK)),
        (
            "/$defs/node/properties/question/pattern",
            Value::from(NOT_BLANK),
        ),
        (
            "/$defs/node/properties/answer/pattern",
            Value::from(NOT_BLANK),
        ),
        ("/$defs/node/properties/id/$ref", Value::from("#/$defs/id")),
        (
            "/$defs/node/properties/depends_on/items/$ref",
            Value::from("#/$defs/id"),
        ),
        (
            "/$defs/node/properties/depends_on/maxItems",
            Value::from(MAX_DEPENDENCIES),
        ),
        (
            "/$defs/node/properties/depends_on/uniqueItems",
            Value::Bool(true),
        ),
        (
            "/$defs/node/properties/confidence/$ref",
            Value::from("#/$defs/probability"),
        ),
        (
            "/$defs/node/properties/is_decision/$ref",
            Value::from("#/$defs/probability"),
        ),
        (
            "/$defs/turn/properties/confidence/$ref",
            Value::from("#/$defs/probability"),
        ),
    ];

    for (pointer, value) in expected {
        assert_eq!(schema.pointer(pointer), Some(&value), "{pointer}");
    }
    Ok(())
}
