//! `anneal playbook show|list` through the built binary: each playbook's text exactly as the
//! file in `crates/anneal-cli/playbooks/`, its media type, and an unknown name as a usage error.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};

use crate::support::anneal_in;

type TestResult = Result<(), Box<dyn Error>>;

/// The playbook files, as committed.
fn playbooks() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("playbooks")
}

/// `anneal` with `args`, in a fresh temporary directory.
fn anneal(args: &[&str]) -> Result<Output, Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    Ok(anneal_in(dir.path()).args(args).output()?)
}

fn envelope(output: &Output) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&output.stdout)?)
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn show_spec_template_writes_the_built_in_text_as_markdown() -> TestResult {
    let file = std::fs::read_to_string(playbooks().join("spec.template.md"))?;

    let output = anneal(&["playbook", "show", "spec-template"])?;

    let envelope = envelope(&output)?;
    assert_eq!(output.status.code(), Some(0), "{envelope}");
    assert_eq!(
        (
            &envelope["ok"],
            &envelope["command"],
            &envelope["data"]["name"],
            &envelope["data"]["media_type"],
        ),
        (
            &json!(true),
            &json!("playbook show"),
            &json!("spec-template"),
            &json!("text/markdown"),
        ),
        "{envelope}"
    );
    assert_eq!(envelope["data"]["content"], json!(file), "exactly the file");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn show_gives_every_playbook_its_file_and_media_type() -> TestResult {
    let expected = [
        ("plan-template", "plan.template.md", "text/markdown"),
        (
            "design-tree-schema",
            "schemas/design-tree.schema.json",
            "application/json",
        ),
        (
            "plan-schema",
            "schemas/plan.schema.json",
            "application/json",
        ),
        ("route", "route.json", "application/json"),
        (
            "clef-thresholds",
            "clef-thresholds.json",
            "application/json",
        ),
    ];
    for (name, file, media_type) in expected {
        let text = std::fs::read_to_string(playbooks().join(file))?;

        let output = anneal(&["playbook", "show", name])?;

        let envelope = envelope(&output)?;
        assert_eq!(output.status.code(), Some(0), "{name}: {envelope}");
        assert_eq!(
            [
                &envelope["data"]["name"],
                &envelope["data"]["media_type"],
                &envelope["data"]["content"],
            ],
            [&json!(name), &json!(media_type), &json!(text)],
            "{name}"
        );
    }
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn an_unknown_name_is_a_usage_error_with_nothing_on_stdout() -> TestResult {
    let output = anneal(&["playbook", "show", "constitution"])?;

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let stderr = String::from_utf8(output.stderr)?;
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(stderr.starts_with("ERROR "), "{stderr}");
    assert!(stderr.contains("constitution"), "names the value: {stderr}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn list_names_every_playbook_in_order_with_its_media_type() -> TestResult {
    let output = anneal(&["playbook", "list"])?;

    let envelope = envelope(&output)?;
    assert_eq!(output.status.code(), Some(0), "{envelope}");
    let listed: Vec<(&str, &str)> = envelope["data"]["playbooks"]
        .as_array()
        .ok_or("data.playbooks is an array")?
        .iter()
        .map(|p| {
            (
                p["name"].as_str().unwrap_or_default(),
                p["media_type"].as_str().unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            ("spec-template", "text/markdown"),
            ("plan-template", "text/markdown"),
            ("design-tree-schema", "application/json"),
            ("plan-schema", "application/json"),
            ("route", "application/json"),
            ("clef-thresholds", "application/json"),
        ],
        "{envelope}"
    );
    Ok(())
}
