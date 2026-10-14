use std::error::Error;
use std::io::Write as _;

use anneal_ledger::event::TokenUsage;
use serde_json::{Value, json};

use super::{
    Activity, Usage, activity_since, last_assistant_text, read_since, read_tail, skills_invoked,
    usage,
};

type TestResult = Result<(), Box<dyn Error>>;

/// `values` as JSONL bytes, as Claude Code writes a transcript.
fn jsonl(values: &[Value]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for value in values {
        bytes.extend(value.to_string().into_bytes());
        bytes.push(b'\n');
    }
    bytes
}

fn assistant(id: &str, content: Value, ts: &str, output: u64) -> Value {
    let mut line = json!({
        "type": "assistant",
        "timestamp": ts,
        "message": {
            "id": id,
            "role": "assistant",
            "usage": {
                "input_tokens": 2,
                "cache_creation_input_tokens": 10,
                "cache_read_input_tokens": 100,
                "output_tokens": output,
            },
        },
    });
    line["message"]["content"] = content;
    line
}

fn user(text: &str, ts: &str) -> Value {
    json!({"type": "user", "timestamp": ts, "message": {"role": "user", "content": text}})
}

#[test]
fn the_last_assistant_text_joins_the_text_blocks_of_its_last_message() {
    let bytes = jsonl(&[
        assistant(
            "m1",
            json!([{"type": "text", "text": "old"}]),
            "2026-10-11T00:00:00Z",
            1,
        ),
        user("next", "2026-10-11T00:00:01Z"),
        assistant(
            "m2",
            json!([{"type": "text", "text": "first"}]),
            "2026-10-11T00:00:02Z",
            1,
        ),
        assistant(
            "m2",
            json!([{"type": "tool_use", "id": "t", "name": "Bash", "input": {}}]),
            "2026-10-11T00:00:02Z",
            1,
        ),
        assistant(
            "m2",
            json!([{"type": "text", "text": "second"}]),
            "2026-10-11T00:00:03Z",
            5,
        ),
    ]);

    assert_eq!(
        last_assistant_text(&bytes).as_deref(),
        Some("first\nsecond"),
        "the text of every line of the last message, in order"
    );
}

#[test]
fn a_cut_first_line_and_non_json_lines_are_passed_over() {
    let mut bytes = b"ext\": \"cut\"}]}}\nnot json\n".to_vec();
    bytes.extend(jsonl(&[assistant(
        "m1",
        json!([{"type": "text", "text": "kept"}]),
        "2026-10-11T00:00:00Z",
        1,
    )]));

    assert_eq!(
        last_assistant_text(&bytes).as_deref(),
        Some("kept"),
        "read on"
    );
    assert_eq!(
        last_assistant_text(b"no json at all"),
        None,
        "nothing to read"
    );
}

#[test]
fn activity_since_an_offset_sees_only_tool_calls_written_after_it() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("t.jsonl");
    std::fs::write(
        &path,
        jsonl(&[assistant(
            "m1",
            json!([{"type": "tool_use", "id": "t", "name": "Bash", "input": {}}]),
            "2026-10-11T00:00:00Z",
            1,
        )]),
    )?;
    let offset = std::fs::metadata(&path)?.len();
    let mut file = std::fs::OpenOptions::new().append(true).open(&path)?;
    file.write_all(&jsonl(&[user(
        "say \"type\":\"tool_use\"",
        "2026-10-11T00:00:01Z",
    )]))?;

    assert_eq!(
        activity_since(&path, offset, 1024)?,
        Activity::Quiet,
        "the earlier call does not count, nor quoted text"
    );

    file.write_all(&jsonl(&[assistant(
        "m2",
        json!([{"type": "tool_use", "id": "u", "name": "Read", "input": {}}]),
        "2026-10-11T00:00:02Z",
        1,
    )]))?;
    assert_eq!(
        activity_since(&path, offset, 1024)?,
        Activity::ToolCall,
        "a new call is seen"
    );
    Ok(())
}

#[test]
fn a_shrunk_or_overgrown_transcript_counts_as_activity() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("t.jsonl");
    std::fs::write(&path, b"0123456789")?;

    assert_eq!(
        activity_since(&path, 20, 1024)?,
        Activity::ToolCall,
        "rewritten (compacted): assume work happened"
    );
    assert_eq!(
        activity_since(&path, 0, 4)?,
        Activity::ToolCall,
        "more appended than the cap: assume work happened"
    );
    Ok(())
}

#[test]
fn read_tail_returns_at_most_the_last_bytes() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("t.jsonl");
    std::fs::write(&path, b"0123456789")?;

    assert_eq!(read_tail(&path, 4)?, b"6789", "the end of the file");
    assert_eq!(read_tail(&path, 64)?, b"0123456789", "all of a short file");
    Ok(())
}

#[test]
fn usage_sums_each_message_once_and_spans_first_to_last_timestamp() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("agent.jsonl");
    std::fs::write(
        &path,
        jsonl(&[
            user("go", "2026-10-11T00:00:00.500Z"),
            assistant(
                "m1",
                json!([{"type": "text", "text": "a"}]),
                "2026-10-11T00:00:01Z",
                3,
            ),
            assistant(
                "m1",
                json!([{"type": "text", "text": "b"}]),
                "2026-10-11T00:00:01Z",
                7,
            ),
            assistant(
                "m2",
                json!([{"type": "text", "text": "c"}]),
                "2026-10-11T00:01:00Z",
                4,
            ),
        ]),
    )?;

    assert_eq!(
        usage(&path, 1 << 20)?,
        Some(Usage {
            duration_ms: Some(59_500),
            tokens: Some(TokenUsage {
                input: 4,
                output: 11,
                cache_read: 200,
                cache_creation: 20,
            }),
            last_text: Some("c".to_owned()),
        }),
        "m1 counted once with its last usage, plus m2"
    );
    Ok(())
}

#[test]
fn usage_of_a_transcript_over_the_cap_is_not_read() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("agent.jsonl");
    std::fs::write(&path, b"0123456789")?;

    assert_eq!(
        usage(&path, 4)?,
        None,
        "too large: no figures, nothing read"
    );
    Ok(())
}

#[test]
fn read_since_returns_what_was_appended_unless_it_shrank_or_overgrew() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("t.jsonl");
    std::fs::write(&path, b"0123456789")?;

    assert_eq!(
        read_since(&path, 6, 64)?,
        Some(b"6789".to_vec()),
        "the new bytes"
    );
    assert_eq!(
        read_since(&path, 20, 64)?,
        None,
        "rewritten: shorter than the offset"
    );
    assert_eq!(read_since(&path, 0, 4)?, None, "grown past the cap");
    Ok(())
}

#[test]
fn skills_invoked_lists_each_skill_tool_call_in_order() {
    let bytes = jsonl(&[
        assistant(
            "m1",
            json!([
                {"type": "text", "text": "loading"},
                {"type": "tool_use", "id": "a", "name": "Skill", "input": {"skill": "anneal:plan"}},
                {"type": "tool_use", "id": "b", "name": "Bash", "input": {"skill": "not a skill"}},
            ]),
            "2026-10-11T00:00:00Z",
            1,
        ),
        user("use the Skill tool with build", "2026-10-11T00:00:01Z"),
        assistant(
            "m2",
            json!([{"type": "tool_use", "id": "c", "name": "Skill", "input": {"skill": "build"}}]),
            "2026-10-11T00:00:02Z",
            1,
        ),
    ]);

    assert_eq!(skills_invoked(&bytes), ["anneal:plan", "build"]);
}
