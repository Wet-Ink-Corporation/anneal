use std::error::Error;
use std::path::Path;

use anneal_ledger::Store;
use serde_json::Value;

use super::{
    PlanBytes, Recording, approval, conflict, latest, normalised, plan_id, read, record, refuse,
};
use crate::exit_codes;

#[test]
fn the_plan_id_drops_only_current_directory_components() {
    assert_eq!(
        plan_id(Path::new("./a/./b.md")).as_deref(),
        Some("a/b.md"),
        "dots"
    );
    assert_eq!(
        plan_id(Path::new("a/../b.md")).as_deref(),
        Some("a/../b.md"),
        "parents kept"
    );
}

#[test]
fn the_plan_id_keeps_the_root_and_drops_empty_components() {
    assert_eq!(
        plan_id(Path::new("/abs/./p.md")).as_deref(),
        Some("/abs/p.md"),
        "absolute"
    );
    assert_eq!(
        plan_id(Path::new("a//b.md")).as_deref(),
        Some("a/b.md"),
        "doubled separator"
    );
}

#[test]
fn the_plan_id_keeps_a_unc_prefix() {
    assert_eq!(
        normalised(r"\\server\share\.\p.md", '\\'),
        "//server/share/p.md",
        "a Windows UNC path"
    );
    assert_eq!(
        normalised("//server/share//p.md", '/'),
        "//server/share/p.md",
        "a leading `//`"
    );
    assert_eq!(
        normalised(r".\docs\p.md", '\\'),
        "docs/p.md",
        "a Windows relative path is the same plan as on unix"
    );
}

/// A plan file in a fresh directory, read as `approve` reads it, and a ledger beside it.
fn setup(dir: &Path) -> Result<(PlanBytes, Store), Box<dyn Error>> {
    let file = dir.join("p.md");
    std::fs::write(&file, "# Plan\n")?;
    let plan = read(&file)?;
    let store = Store::open(&dir.join("ledger.sqlite3"))?;
    Ok((plan, store))
}

/// `plan` with other content: the same id, another hash.
fn edited(plan: &PlanBytes) -> PlanBytes {
    // A second, owned plan: both are used side by side.
    PlanBytes {
        id: plan.id.clone(),
        bytes: b"# Plan, edited\n".to_vec(),
        hash: "sha256:other".to_owned(),
        approvals: plan.approvals.clone(),
    }
}

#[test]
fn an_approval_with_another_hash_landing_after_the_read_is_a_conflict() -> Result<(), Box<dyn Error>>
{
    let dir = tempfile::tempdir()?;
    let (plan, store) = setup(dir.path())?;
    let previous = latest(&store, &plan.approvals)?;
    store.append(&approval(&edited(&plan), None)?)?;

    let recording = record(&store, &approval(&plan, None)?, &plan, previous.as_ref())?;

    assert_eq!(recording, Recording::Conflict, "the other approval won");
    assert_eq!(
        store.query(&plan.approvals)?.len(),
        1,
        "nothing appended on a conflict"
    );
    Ok(())
}

#[test]
fn an_approval_with_the_same_hash_landing_after_the_read_is_already_approved()
-> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let (plan, store) = setup(dir.path())?;
    let previous = latest(&store, &plan.approvals)?;
    let winner = store.append(&approval(&plan, None)?)?;

    let recording = record(&store, &approval(&plan, None)?, &plan, previous.as_ref())?;

    assert_eq!(
        recording,
        Recording::Already(winner),
        "the same approval won"
    );
    assert_eq!(store.query(&plan.approvals)?.len(), 1, "nothing appended");
    Ok(())
}

#[test]
fn an_approval_at_or_before_the_one_read_does_not_conflict() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let (plan, store) = setup(dir.path())?;
    let earlier = store.append(&approval(&edited(&plan), None)?)?;
    let previous = latest(&store, &plan.approvals)?;
    assert_eq!(
        previous.as_ref().map(|recorded| recorded.position),
        Some(earlier),
        "the read sees the earlier approval"
    );

    let recording = record(&store, &approval(&plan, None)?, &plan, previous.as_ref())?;

    assert!(
        matches!(recording, Recording::Appended(position) if position > earlier),
        "appended after it: {recording:?}"
    );
    Ok(())
}

#[test]
fn a_conflict_is_refused_with_exit_3_and_a_conflict_key() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let (plan, _store) = setup(dir.path())?;
    let mut out = Vec::new();

    let code = refuse(&plan, None, &conflict(&plan), &mut out)?;

    assert_eq!(code, exit_codes::PLAN_NOT_APPROVED, "exit 3");
    let envelope: Value = serde_json::from_slice(&out)?;
    let keys: Vec<&str> = envelope
        .get("findings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|finding| finding.get("key").and_then(Value::as_str))
        .collect();
    assert_eq!(
        keys,
        vec![format!("plan/approve/conflict:{}", plan.id)],
        "one conflict finding: {envelope}"
    );
    assert_eq!(envelope.get("ok"), Some(&Value::Bool(false)), "not ok");
    Ok(())
}
