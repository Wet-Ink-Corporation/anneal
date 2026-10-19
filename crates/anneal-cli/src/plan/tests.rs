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

#[test]
fn losing_a_race_to_the_same_hash_by_another_approver_appends_after_it()
-> Result<(), Box<dyn Error>> {
    use anneal_ledger::event::{Actor, By, Via};

    let dir = tempfile::tempdir()?;
    let (plan, store) = setup(dir.path())?;
    let model = By::known(Some(Actor::Model), Some(Via::Bash));
    let human = By::known(Some(Actor::Human), Some(Via::Terminal));
    let previous = latest(&store, &plan.approvals)?;
    let winner = store.append(&super::approval_by(&plan, None, model)?)?;

    let recording = record(
        &store,
        &super::approval_by(&plan, None, human)?,
        &plan,
        previous.as_ref(),
    )?;

    assert!(
        matches!(recording, Recording::Appended(position) if position > winner),
        "the human's approval is news, not a conflict: {recording:?}"
    );
    let approvals = store.query(&plan.approvals)?;
    let givers: Vec<Option<By>> = approvals
        .iter()
        .filter_map(|recorded| super::approved(&recorded.event).map(|approval| approval.by))
        .collect();
    assert_eq!(givers, [model, human], "both kept, the human's latest");
    Ok(())
}

#[test]
fn losing_a_race_to_another_hash_after_an_approver_race_is_still_a_conflict()
-> Result<(), Box<dyn Error>> {
    use anneal_ledger::event::{Actor, By, Via};

    let dir = tempfile::tempdir()?;
    let (plan, store) = setup(dir.path())?;
    let previous = latest(&store, &plan.approvals)?;
    store.append(&super::approval_by(
        &plan,
        None,
        By::known(Some(Actor::Model), None),
    )?)?;
    store.append(&approval(&edited(&plan), None)?)?;

    let recording = record(
        &store,
        &super::approval_by(&plan, None, By::known(Some(Actor::Human), Some(Via::Mod)))?,
        &plan,
        previous.as_ref(),
    )?;

    assert_eq!(
        recording,
        Recording::Conflict,
        "the edited plan's approval is latest"
    );
    assert_eq!(store.query(&plan.approvals)?.len(), 2, "nothing appended");
    Ok(())
}

/// A ledger on which every conditioned append loses to a new approval of the same bytes by
/// another approver, as if model approvals kept landing first.
struct AlwaysOutraced {
    /// The approval that keeps winning, re-read at a new position each time.
    winner: anneal_ledger::EventEnvelope,
    /// How many conditioned appends were tried.
    tries: std::cell::Cell<usize>,
}

impl super::ApprovalLedger for AlwaysOutraced {
    fn append_if(
        &self,
        _event: &anneal_ledger::EventEnvelope,
        _condition: &anneal_ledger::Condition,
    ) -> Result<anneal_ledger::Appended, crate::error::CliError> {
        self.tries.set(self.tries.get().saturating_add(1));
        Ok(anneal_ledger::Appended::Conflict)
    }

    fn latest(
        &self,
        _approvals: &anneal_ledger::Filter,
    ) -> Result<Option<anneal_ledger::Recorded>, crate::error::CliError> {
        let position = u64::try_from(self.tries.get()).unwrap_or(u64::MAX);
        // Each read hands back an owned event, as the store's does.
        Ok(Some(anneal_ledger::Recorded {
            position,
            event: self.winner.clone(),
        }))
    }
}

#[test]
fn an_approval_that_keeps_losing_to_other_approvers_gives_up_as_a_conflict_after_8_races()
-> Result<(), Box<dyn Error>> {
    use anneal_ledger::event::{Actor, By, Via};

    let dir = tempfile::tempdir()?;
    let (plan, _store) = setup(dir.path())?;
    let model = By::known(Some(Actor::Model), Some(Via::Bash));
    let ledger = AlwaysOutraced {
        winner: super::approval_by(&plan, None, model)?,
        tries: std::cell::Cell::new(0),
    };
    let human = By::known(Some(Actor::Human), Some(Via::Terminal));

    let recording = record(
        &ledger,
        &super::approval_by(&plan, None, human)?,
        &plan,
        None,
    )?;

    assert_eq!(recording, Recording::Conflict, "it gave up");
    assert_eq!(
        ledger.tries.get(),
        super::MAX_APPROVER_RACES,
        "one conditioned append per race, no more"
    );
    Ok(())
}
