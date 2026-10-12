//! `anneal hook injection-scan` through the built binary: the fixture corpus (every positive
//! flagged, no benign lookalike flagged), and the hook never blocks, whatever its input or
//! its stdout.

use std::error::Error;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};

use serde_json::{Value, json};

use crate::support::anneal_in;

type TestResult = Result<(), Box<dyn Error>>;

/// The corpus: `positive/` and `benign/`.
fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/injection")
}

/// Runs `anneal hook injection-scan` with `stdin`, its stdout closed first when `close_stdout`.
fn run_hook(stdin: &[u8], close_stdout: bool) -> Result<Output, Box<dyn Error>> {
    let scratch = tempfile::tempdir()?;
    let mut child = anneal_in(scratch.path())
        .args(["hook", "injection-scan"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if close_stdout {
        drop(child.stdout.take());
    }
    let mut input = child.stdin.take().ok_or("stdin is piped")?;
    match input.write_all(stdin) {
        // The hook stops reading at its cap; the exit code below still reports the run.
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {}
        written => written?,
    }
    drop(input);
    Ok(child.wait_with_output()?)
}

/// The hook's stdout as JSON, after checking it exited 0 and wrote at most one line with no
/// `decision` and no `continue`.
fn never_blocking(output: &Output) -> Result<Option<Value>, Box<dyn Error>> {
    assert_eq!(output.status.code(), Some(0), "fails open: {output:?}");
    let stdout = std::str::from_utf8(&output.stdout)?;
    if stdout.is_empty() {
        return Ok(None);
    }
    assert_eq!(stdout.lines().count(), 1, "one line: {stdout}");
    let value: Value = serde_json::from_str(stdout)?;
    assert!(
        value.get("decision").is_none() && value.get("continue").is_none(),
        "never blocks: {value}"
    );
    Ok(Some(value))
}

/// The hook input for `file`: a `WebFetch` result for `.html`, else a `Read` result.
fn input_for(file: &Path) -> Result<Value, Box<dyn Error>> {
    let text = std::fs::read_to_string(file)?;
    let name = file.display().to_string();
    let call = if file.extension().is_some_and(|ext| ext == "html") {
        json!({"tool_name": "WebFetch",
               "tool_input": {"url": format!("https://example.com/{name}"), "prompt": "read"},
               "tool_response": {"code": 200, "result": text}})
    } else {
        json!({"tool_name": "Read",
               "tool_input": {"file_path": name},
               "tool_response": {"type": "text", "file": {"filePath": name, "content": text,
                                                          "startLine": 1}}})
    };
    Ok(json!({"hook_event_name": "PostToolBatch", "cwd": ".", "tool_calls": [call]}))
}

/// The names of the files of `dir`, sorted, each with whether the hook flagged it.
fn flagged(dir: &Path) -> Result<Vec<(String, bool)>, Box<dyn Error>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<_, _>>()?;
    files.sort();
    let mut flagged = Vec::new();
    for file in files {
        let name = file
            .file_name()
            .ok_or("a file name")?
            .to_string_lossy()
            .into_owned();
        let output = run_hook(input_for(&file)?.to_string().as_bytes(), false)?;
        let hit = match never_blocking(&output)? {
            Some(value) => {
                let context = value
                    .get("hookSpecificOutput")
                    .and_then(|specific| specific.get("additionalContext"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("{name}: a hit is additionalContext: {value}"))?;
                assert!(
                    context.starts_with("anneal injection-scan: ") && !context.contains('\n'),
                    "{name}: one note line: {context}"
                );
                true
            }
            None => false,
        };
        flagged.push((name, hit));
    }
    Ok(flagged)
}

/// The names in `flagged` whose flag is `hit`.
fn names_with(flagged: &[(String, bool)], hit: bool) -> Vec<&str> {
    flagged
        .iter()
        .filter(|(_, flag)| *flag == hit)
        .map(|(name, _)| name.as_str())
        .collect()
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn every_positive_fixture_is_flagged_and_no_benign_lookalike_is() -> TestResult {
    let positives = flagged(&corpus().join("positive"))?;
    let benign = flagged(&corpus().join("benign"))?;

    assert!(
        positives.len() >= 10 && benign.len() >= 10,
        "at least 10 fixtures of each: {} positive, {} benign",
        positives.len(),
        benign.len()
    );
    let missed = names_with(&positives, false);
    let false_positives = names_with(&benign, true);
    assert_eq!(
        (missed.as_slice(), false_positives.as_slice()),
        (&[][..], &[][..]),
        "true positives {}/{}, false positives {}/{}",
        positives.len().saturating_sub(missed.len()),
        positives.len(),
        false_positives.len(),
        benign.len()
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn input_that_is_not_json_is_one_error_line_and_exits_0() -> TestResult {
    let output = run_hook(b"\x00\xffnot json", false)?;

    let value = never_blocking(&output)?.ok_or("an ERROR line as the systemMessage")?;
    let message = value["systemMessage"].as_str().ok_or("a systemMessage")?;
    assert!(message.starts_with("ERROR injection-scan: "), "{message}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn input_over_the_cap_exits_0_without_blocking() -> TestResult {
    let huge = vec![b' '; 16 * 1024 * 1024 + 2];

    let output = run_hook(&huge, false)?;

    let value = never_blocking(&output)?.ok_or("an ERROR line as the systemMessage")?;
    let message = value["systemMessage"].as_str().ok_or("a systemMessage")?;
    assert!(message.contains("larger than"), "{message}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_closed_stdout_still_exits_0_with_one_error_line() -> TestResult {
    let input = input_for(&corpus().join("positive/p01-ignore-previous.md"))?;

    let output = run_hook(input.to_string().as_bytes(), true)?;

    assert_eq!(output.status.code(), Some(0), "fails open: {output:?}");
    let err = String::from_utf8(output.stderr)?;
    assert!(
        err.starts_with("ERROR writing output") && err.lines().count() == 1,
        "one ERROR line says why: {err}"
    );
    Ok(())
}
