use anneal_ledger::event::{Actor, By, Verdict, VerdictSet, VerdictSource, Via};

use super::{Fresh, Head, Invalid, Outcome, Record, assess, needs_patch_id};

const HEAD: &str = "1111111111111111111111111111111111111111";
const OLD: &str = "2222222222222222222222222222222222222222";
const PATCH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_PATCH: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn set(verdict: Verdict, source: VerdictSource, sha: &str, patch_id: &str) -> VerdictSet {
    VerdictSet {
        pr: 7,
        verdict,
        source: Some(source),
        sha: Some(sha.to_owned()),
        patch_id: Some(patch_id.to_owned()),
        base: Some("main".to_owned()),
        by: None,
    }
}

fn records(sets: &[VerdictSet]) -> Vec<Record<'_>> {
    sets.iter()
        .zip(1_u64..)
        .map(|(set, position)| Record { position, set })
        .collect()
}

fn head(patch_id: Option<&'static str>) -> Head<'static> {
    Head {
        sha: HEAD,
        base: Some("main"),
        patch_id,
    }
}

fn outcome(sets: &[VerdictSet], patch_id: Option<&'static str>) -> Outcome {
    assess(&records(sets), &head(patch_id), false).outcome()
}

/// A fresh human `approved` at the head, given by `actor` through `via`.
fn human_approval(actor: Option<Actor>, via: Option<Via>) -> VerdictSet {
    VerdictSet {
        by: By::known(actor, via),
        ..set(Verdict::Approved, VerdictSource::Human, HEAD, PATCH)
    }
}

/// Whether `set` alone is the review while the plugin is not required.
fn is_review(set: VerdictSet) -> bool {
    let sets = [set];
    assess(&records(&sets), &head(None), false).review.is_some()
}

#[test]
fn with_no_verdict_the_pull_request_is_invalid() {
    assert_eq!(outcome(&[], None), Outcome::Invalid(Invalid::NoVerdict));
}

#[test]
fn a_level_set_for_the_head_commit_is_valid() {
    let sets = [set(
        Verdict::TestsVerified,
        VerdictSource::Local,
        HEAD,
        PATCH,
    )];

    let assessment = assess(&records(&sets), &head(None), false);

    assert_eq!(assessment.outcome(), Outcome::Valid);
    assert_eq!(
        assessment.tests.map(|judged| judged.fresh),
        Some(Fresh::BySha),
        "fresh by its commit"
    );
}

#[test]
fn after_a_rebase_only_change_the_verdict_stays_valid_by_patch_id() {
    let sets = [set(
        Verdict::TestsVerified,
        VerdictSource::Local,
        OLD,
        PATCH,
    )];

    let assessment = assess(&records(&sets), &head(Some(PATCH)), false);

    assert_eq!(
        (
            assessment.outcome(),
            assessment.tests.map(|judged| judged.fresh)
        ),
        (Outcome::Valid, Some(Fresh::ByPatchId)),
        "same patch id from the same base"
    );
}

#[test]
fn after_a_real_change_the_verdict_is_stale() {
    let sets = [set(Verdict::Approved, VerdictSource::Human, OLD, PATCH)];

    let assessment = assess(&records(&sets), &head(Some(OTHER_PATCH)), false);

    assert_eq!(
        (assessment.outcome(), assessment.stale),
        (Outcome::Invalid(Invalid::Stale), 1),
        "another patch id"
    );
}

#[test]
fn a_patch_id_taken_from_another_base_does_not_make_a_verdict_fresh() {
    let mut other_base = set(Verdict::TestsVerified, VerdictSource::Local, OLD, PATCH);
    other_base.base = Some("develop".to_owned());

    assert_eq!(
        outcome(&[other_base], Some(PATCH)),
        Outcome::Invalid(Invalid::Stale),
        "patch ids compare only from the same base"
    );
}

#[test]
fn a_verdict_without_a_commit_is_never_fresh() {
    let bare = VerdictSet {
        pr: 7,
        verdict: Verdict::Approved,
        source: None,
        sha: None,
        patch_id: None,
        base: None,
        by: None,
    };

    assert_eq!(
        outcome(&[bare], Some(PATCH)),
        Outcome::Invalid(Invalid::Stale)
    );
}

#[test]
fn the_latest_fresh_verdict_decides_noise_only_and_changes_requested() {
    let noise = [
        set(Verdict::TestsVerified, VerdictSource::Local, HEAD, PATCH),
        set(Verdict::NoiseOnly, VerdictSource::ReviewPlugin, HEAD, PATCH),
    ];
    let changes = [
        set(Verdict::Approved, VerdictSource::Human, HEAD, PATCH),
        set(
            Verdict::ChangesRequested,
            VerdictSource::ReviewPlugin,
            HEAD,
            PATCH,
        ),
    ];

    assert_eq!(
        (outcome(&noise, None), outcome(&changes, None)),
        (
            Outcome::NoiseOnly,
            Outcome::Invalid(Invalid::ChangesRequested)
        ),
        "exit 3 and exit 1"
    );
}

#[test]
fn changes_requested_voids_the_fresh_verdicts_before_it_but_not_after() {
    let sets = [
        set(Verdict::Approved, VerdictSource::Human, HEAD, PATCH),
        set(Verdict::ChangesRequested, VerdictSource::Human, HEAD, PATCH),
        set(Verdict::TestsVerified, VerdictSource::Local, HEAD, PATCH),
    ];

    let assessment = assess(&records(&sets), &head(None), false);

    assert_eq!(
        (
            assessment.outcome(),
            assessment.tests.map(|judged| judged.record.position),
            assessment.review.is_some(),
        ),
        (Outcome::Valid, Some(3), false),
        "the approval before the changes request no longer counts"
    );
}

#[test]
fn changes_requested_voids_an_earlier_human_actor_approval() {
    let sets = [
        human_approval(Some(Actor::Human), None),
        set(Verdict::ChangesRequested, VerdictSource::Human, HEAD, PATCH),
        set(Verdict::TestsVerified, VerdictSource::Local, HEAD, PATCH),
    ];

    let assessment = assess(&records(&sets), &head(None), false);

    assert!(
        assessment.review.is_none(),
        "the approval before the changes request no longer counts: {:?}",
        assessment.review.map(|judged| judged.record.position)
    );
}

#[test]
fn changes_requested_voids_an_earlier_review_plugin_approval() {
    let sets = [
        set(Verdict::Approved, VerdictSource::ReviewPlugin, HEAD, PATCH),
        set(
            Verdict::ChangesRequested,
            VerdictSource::ReviewPlugin,
            HEAD,
            PATCH,
        ),
        set(Verdict::TestsVerified, VerdictSource::Local, HEAD, PATCH),
    ];

    let assessment = assess(&records(&sets), &head(None), true);

    assert!(
        assessment.review.is_none(),
        "the plugin's approval before the changes request no longer counts: {:?}",
        assessment.review.map(|judged| judged.record.position)
    );
}

#[test]
fn a_later_tests_verified_keeps_an_earlier_fresh_approval_as_the_review() {
    let sets = [
        human_approval(Some(Actor::Human), None),
        set(Verdict::TestsVerified, VerdictSource::Local, HEAD, PATCH),
    ];

    let assessment = assess(&records(&sets), &head(None), false);

    assert_eq!(
        assessment.review.map(|judged| judged.record.position),
        Some(1),
        "re-running the tests does not drop the review"
    );
}

#[test]
fn a_human_approval_is_a_review_only_while_the_plugin_is_not_required() {
    let sets = [human_approval(Some(Actor::Human), None)];

    let interim = assess(&records(&sets), &head(None), false);
    let plugin_required = assess(&records(&sets), &head(None), true);

    assert_eq!(
        (interim.review.is_some(), plugin_required.review.is_some()),
        (true, false),
        "the interim rule (Ryan, 2026-10-11) and its switch"
    );
}

#[test]
fn a_human_approval_given_by_a_human_actor_counts_as_the_review() {
    assert!(
        is_review(human_approval(Some(Actor::Human), None)),
        "by.actor human (wi-10f284)"
    );
}

#[test]
fn a_human_approval_given_by_a_model_is_no_review() {
    assert!(
        !is_review(human_approval(Some(Actor::Model), None)),
        "by.actor model (wi-10f284)"
    );
}

#[test]
fn a_human_approval_with_no_actor_is_no_review() {
    assert_eq!(
        (
            is_review(human_approval(None, None)),
            is_review(human_approval(None, Some(Via::Terminal))),
        ),
        (false, false),
        "no by, and a by with only a via: a legacy approval or one set without --actor"
    );
}

#[test]
fn via_does_not_decide_whether_a_human_approval_counts() {
    let vias = [None, Some(Via::Mod), Some(Via::Bash), Some(Via::Terminal)];

    let counted = vias.map(|via| {
        (
            is_review(human_approval(Some(Actor::Human), via)),
            is_review(human_approval(Some(Actor::Model), via)),
        )
    });

    assert_eq!(counted, [(true, false); 4], "via is for audit only");
}

#[test]
fn which_verdicts_count_as_a_review() {
    let cases = [
        (Verdict::Reviewed, VerdictSource::ReviewPlugin, true),
        (Verdict::Approved, VerdictSource::ReviewPlugin, true),
        (Verdict::NoiseOnly, VerdictSource::ReviewPlugin, true),
        (Verdict::TestsVerified, VerdictSource::ReviewPlugin, false),
        (Verdict::Reviewed, VerdictSource::Human, false),
        (Verdict::Approved, VerdictSource::Local, false),
        (Verdict::Reviewed, VerdictSource::Local, false),
    ];

    for (verdict, source, counts) in cases {
        let sets = [set(verdict, source, HEAD, PATCH)];
        let assessment = assess(&records(&sets), &head(None), true);
        assert_eq!(
            assessment.review.is_some(),
            counts,
            "{verdict:?} from {source:?} with the plugin required"
        );
    }
}

#[test]
fn a_patch_id_is_needed_only_for_a_verdict_on_another_commit_from_the_same_base() {
    let at_head = [set(Verdict::Approved, VerdictSource::Human, HEAD, PATCH)];
    let at_old = [set(Verdict::Approved, VerdictSource::Human, OLD, PATCH)];

    assert_eq!(
        (
            needs_patch_id(&records(&at_head), HEAD, "main"),
            needs_patch_id(&records(&at_old), HEAD, "main"),
            needs_patch_id(&records(&at_old), HEAD, "develop"),
        ),
        (false, true, false),
        "git is asked only when a patch id could make a difference"
    );
}
