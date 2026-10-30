//! `human-only` (#62): a model's Bash call may not record a human verdict or label, however the
//! shell forms the words; `plan approve` (any actor), `plan check` and `verdict valid` stay
//! allowed.

use super::forms::{forms, still_runs};
use super::{ALLOW, assert_table, rule_for};
use crate::hook::guard::Rule;

const HUMAN_ONLY: Option<Rule> = Some(Rule::HumanOnly);

/// A verdict a model may record, until `--actor human` is added.
const SET: &str = "anneal verdict set --pr 7 --sha HEAD --level approved --source human";

/// The human verdict and label, as `forms` wraps them.
const BASES: [&str; 2] = [
    "anneal verdict set --pr 7 --level approved --source human --actor human",
    "anneal decide label d1-2-3 yes --actor human",
];

#[test]
fn a_human_verdict_or_label_stays_human_only_in_every_wrapping_form() {
    let missed: Vec<String> = BASES
        .iter()
        .flat_map(|base| {
            forms()
                .into_iter()
                .map(move |(name, form)| (name, form(base)))
        })
        .filter_map(|(name, command)| {
            let actual = rule_for(&command);
            (actual != HUMAN_ONLY).then(|| format!("{name}: {command:?} -> {actual:?}"))
        })
        .collect();

    assert!(missed.is_empty(), "misjudged:\n{}", missed.join("\n"));
}

#[test]
fn a_human_verdict_or_label_stays_denied_in_every_pair_of_forms() {
    let forms = forms();
    let mut missed = Vec::new();
    for base in BASES {
        for (inner_name, inner) in &forms {
            let inner = inner(base);
            for (outer_name, outer) in &forms {
                if !still_runs(outer_name, &inner) {
                    continue;
                }
                let command = outer(&inner);
                if rule_for(&command).is_none() {
                    missed.push(format!("{outer_name} of {inner_name}: {command:?}"));
                }
            }
        }
    }

    assert!(missed.is_empty(), "allowed:\n{}", missed.join("\n"));
}

#[test]
fn a_human_verdict_or_label_is_denied_however_the_shell_forms_it() {
    let rows: Vec<(String, Option<Rule>)> = [
        format!("{SET} --actor human"),
        format!("{SET} --actor=human --via mod"),
        format!("{SET} --via terminal --actor human"),
        format!("{SET} '--actor' \"human\""),
        format!("{SET} --actor h{{u,}}man"),
        format!("{SET} --actor \"$WHO\""),
        format!("{SET} --actor=\"$WHO\""),
        format!("{SET} $FLAGS"),
        format!("{SET} \"$flag\" human"),
        format!("who=human; {SET} --actor $who"),
        format!("/opt/plugin/libexec/{SET} --actor human"),
        "\"${CLAUDE_PLUGIN_ROOT}/libexec/anneal\" verdict set --pr 7 --actor human".to_owned(),
        "\"$ANNEAL\" verdict set --pr 7 --actor human".to_owned(),
        format!("bash -c '{SET} --actor human'"),
        format!("sh -c \"{SET} --actor human\""),
        format!("eval '{SET} --actor human'"),
        format!("CI=1 {SET} --actor human"),
        format!("env A=1 {SET} --actor human"),
        format!("sudo {SET} --actor human"),
        format!("caffeinate {SET} --actor human"),
        format!("git status && {SET} --actor human"),
        format!("true; {SET} --actor human"),
        format!("false || {SET} --actor human"),
        format!("echo hi | {SET} --actor human"),
        "anneal verdict \"$sub\" --pr 7 --actor human".to_owned(),
        "anneal decide label d1-2-3 yes --actor human".to_owned(),
        "anneal decide label --actor=human d1-2-3 yes".to_owned(),
        "bash -c 'anneal decide label d1-2-3 yes --actor human --via mod'".to_owned(),
        "anneal decide label \"$id\" \"$outcome\" yes".to_owned(),
        "echo --actor human | xargs anneal verdict set --pr 7".to_owned(),
        "cat flags | xargs anneal verdict set --pr 7".to_owned(),
    ]
    .into_iter()
    .map(|command| (command, HUMAN_ONLY))
    .collect();
    let rows: Vec<(&str, Option<Rule>)> = rows
        .iter()
        .map(|(command, rule)| (command.as_str(), *rule))
        .collect();

    assert_table(&rows);
}

#[test]
fn plan_approvals_checks_valid_and_other_actors_stay_allowed() {
    assert_table(&[
        (
            "anneal plan approve docs/plans/x.md --actor human --via bash",
            ALLOW,
        ),
        (
            "anneal plan approve docs/plans/x.md --actor model --via bash",
            ALLOW,
        ),
        (
            "anneal plan approve docs/plans/x.md --actor \"$WHO\"",
            ALLOW,
        ),
        ("anneal plan check docs/plans/x.md", ALLOW),
        ("anneal verdict valid --pr 7", ALLOW),
        ("anneal verdict valid --pr 7 --head \"$HEAD_SHA\"", ALLOW),
        (
            "anneal verdict set --pr 7 --sha HEAD --level tests-verified --source local \
             --actor model --via bash",
            ALLOW,
        ),
        (
            "anneal verdict set --pr \"$PR\" --sha \"$SHA\" --level tests-verified --source \
             local --base \"$BASE\" --session \"$S\" --actor model",
            ALLOW,
        ),
        (
            "anneal verdict set --pr 7 --sha HEAD --level tests-verified --source local \
             --actor=model",
            ALLOW,
        ),
        (
            "anneal verdict set --pr 7 --sha HEAD -- --actor human",
            ALLOW,
        ),
        ("anneal decide label d1-2-3 yes", ALLOW),
        (
            "anneal decide label d1-2-3 yes --actor model --via mod",
            ALLOW,
        ),
        ("anneal decide label \"$id\" yes --actor model", ALLOW),
        ("anneal decide label \"$id\" yes", ALLOW),
        ("anneal decide label d1-2-3 -- --actor human", ALLOW),
        ("anneal ledger query --type verdict_set", ALLOW),
        ("echo anneal verdict set --pr 7 --actor human", ALLOW),
        ("grep -r 'anneal verdict set --actor human' docs", ALLOW),
        ("anneal-other verdict set --pr 7 --actor human", ALLOW),
    ]);
}

#[test]
fn a_subcommand_path_known_only_at_run_time_is_denied_with_a_literal_human_actor() {
    assert_table(&[
        ("anneal \"$@\" --actor human", HUMAN_ONLY),
        ("anneal $SUB --actor human", HUMAN_ONLY),
        ("anneal $SUB --actor=human", HUMAN_ONLY),
        ("anneal \"$@\" verdict set --pr 7 --actor human", HUMAN_ONLY),
        ("anneal verdict \"$@\" --pr 7 --actor human", HUMAN_ONLY),
        (
            "anneal verdict \"$@\" --sha HEAD --level approved --actor human --via mod",
            HUMAN_ONLY,
        ),
        (
            "f() { anneal \"$@\"; }; f verdict set --pr 7 --actor human",
            HUMAN_ONLY,
        ),
        (
            "f() { anneal \"$@\" --pr 7; }; f verdict set --actor=human",
            HUMAN_ONLY,
        ),
    ]);
}

#[test]
fn a_subcommand_path_known_only_at_run_time_stays_allowed_without_a_human_actor() {
    assert_table(&[
        ("anneal \"$@\"", ALLOW),
        ("anneal $SUB --actor model", ALLOW),
        ("anneal \"$@\" --pr 7", ALLOW),
        ("anneal $SUB --actor humans", ALLOW),
        (
            "f() { anneal \"$@\"; }; f plan check docs/plans/x.md",
            ALLOW,
        ),
    ]);
}

#[test]
fn an_anneal_program_name_is_matched_ignoring_ascii_case_and_a_trailing_exe() {
    assert_table(&[
        ("ANNEAL verdict set --pr 7 --actor human", HUMAN_ONLY),
        ("Anneal verdict set --pr 7 --actor human", HUMAN_ONLY),
        ("anneal.EXE verdict set --pr 7 --actor human", HUMAN_ONLY),
        (
            "/opt/x/Anneal.Exe decide label d1-2-3 yes --actor human",
            HUMAN_ONLY,
        ),
        (
            "'C:\\tools\\ANNEAL.exe' verdict set --pr 7 --actor human",
            HUMAN_ONLY,
        ),
        ("\"$D\"/ANNEAL verdict set --pr 7 --actor human", HUMAN_ONLY),
        (
            "caffeinate ANNEAL verdict set --pr 7 --actor human",
            HUMAN_ONLY,
        ),
        ("ANNEAL verdict set --pr 7 --actor model", ALLOW),
        ("ANNEAL-other verdict set --pr 7 --actor human", ALLOW),
    ]);
}

#[test]
fn a_literal_human_actor_before_an_unread_anneal_path_is_denied() {
    assert_table(&[
        (
            "bash -c 'anneal \"$@\"' _ verdict set --pr 7 --actor human",
            HUMAN_ONLY,
        ),
        (
            "bash -c 'anneal $0 \"$@\"' verdict set --actor human",
            HUMAN_ONLY,
        ),
        (
            "set -- verdict set --pr 7 --actor human; anneal \"$@\"",
            HUMAN_ONLY,
        ),
        (
            "args=(verdict set --pr 7 --actor human); anneal \"${args[@]}\"",
            HUMAN_ONLY,
        ),
        (
            "bash -c 'anneal \"$@\"' _ decide label d1-2-3 yes --actor=human",
            HUMAN_ONLY,
        ),
        (
            "bash -c 'set -- verdict set --actor human; anneal \"$@\"'",
            HUMAN_ONLY,
        ),
        (
            "set -- verdict set --actor human; bash -c 'anneal \"$@\"' _ \"$@\"",
            HUMAN_ONLY,
        ),
        (
            "args=(decide label d1-2-3 yes --actor=human) && sudo anneal \"${args[@]}\"",
            HUMAN_ONLY,
        ),
    ]);
}

#[test]
fn a_literal_human_actor_with_operands_known_only_at_run_time_is_denied() {
    assert_table(&[
        (
            "v() { anneal decide label \"$@\"; }; v d1 yes --actor human",
            HUMAN_ONLY,
        ),
        (
            "set -- d1 yes --actor human; anneal decide label \"$@\"",
            HUMAN_ONLY,
        ),
        (
            "bash -c 'anneal decide label \"$@\"' _ d1 yes --actor human",
            HUMAN_ONLY,
        ),
        (
            "args=(d1 yes --actor human); anneal decide label \"${args[@]}\"",
            HUMAN_ONLY,
        ),
        (
            "su root -c 'anneal decide label \"$@\"' -- d1 yes --actor human",
            HUMAN_ONLY,
        ),
        ("anneal decide label $ARGS \"$flag\"", HUMAN_ONLY),
        (
            "set -- d1 yes --actor human; anneal decide label \"${x:-$@}\"",
            HUMAN_ONLY,
        ),
        (
            "set -- d1 yes --actor human; anneal decide label \"${x-\"$@\"}\"",
            HUMAN_ONLY,
        ),
        (
            "set -- d1 yes --actor human; anneal decide label \"${x:+$@}\"",
            HUMAN_ONLY,
        ),
        (
            "f() { anneal decide label \"${x:-$@}\"; }; f d1 yes --actor human",
            HUMAN_ONLY,
        ),
    ]);
}

#[test]
fn a_human_actor_known_only_at_run_time_reaching_an_unread_call_is_denied() {
    assert_table(&[
        (
            "v() { anneal decide label \"$@\"; }; v d1 yes --actor \"$(echo human)\"",
            HUMAN_ONLY,
        ),
        (
            "set -- d1 yes --actor \"${H:-human}\"; anneal decide label \"$@\"",
            HUMAN_ONLY,
        ),
        (
            "set -- d1 yes \"--actor=${H:-human}\"; anneal decide label \"$@\"",
            HUMAN_ONLY,
        ),
        (
            "a=(d1 yes --actor \"$(echo human)\"); anneal decide label \"${a[@]}\"",
            HUMAN_ONLY,
        ),
        (
            "bash -c 'anneal decide label \"$@\"' _ d1 yes --actor \"$(echo human)\"",
            HUMAN_ONLY,
        ),
        (
            "f() { anneal \"$@\"; }; f verdict set --actor \"$(echo human)\"",
            HUMAN_ONLY,
        ),
        (
            "v() { anneal decide label --via \"$@\"; }; v mod d1 yes --actor \"$(echo human)\"",
            HUMAN_ONLY,
        ),
        ("anneal \"$@\" --actor $WHO", HUMAN_ONLY),
        (
            "f() { anneal \"$@\"; }; f verdict set --actor $WHO",
            HUMAN_ONLY,
        ),
        (
            "v() { anneal decide label \"$@\"; }; v d1 yes --actor $(cat who.txt)",
            HUMAN_ONLY,
        ),
        (
            "set -- d1 yes --actor $WHO; anneal decide label \"$@\"",
            HUMAN_ONLY,
        ),
        (
            "a=(d1 yes --actor $WHO); anneal decide label \"${a[@]}\"",
            HUMAN_ONLY,
        ),
        (
            "bash -c 'anneal decide label \"$@\"' _ d1 yes --actor $WHO",
            HUMAN_ONLY,
        ),
        (
            "v() { anneal decide label \"$@\"; }; v d1 yes --actor \"${WHO@L}\"",
            HUMAN_ONLY,
        ),
    ]);
}

#[test]
fn a_flag_half_that_is_not_a_literal_actor_is_the_documented_limit() {
    // docs/guard.md, Limits: only a literal `--actor` is the flag half beside an unread call.
    // A later change that denies this row is an intended flip.
    assert_table(&[(
        "v() { anneal decide label \"$@\"; }; v d1 yes \"$F\" human",
        ALLOW,
    )]);
}

#[test]
fn the_flag_and_the_human_value_split_across_commands_of_one_call_are_denied() {
    assert_table(&[
        (
            "set -- d1 yes --actor; set -- \"$@\" human; anneal decide label \"$@\"",
            HUMAN_ONLY,
        ),
        (
            "a[0]=d1; a[1]=yes; a[2]=--actor; a[3]=human; anneal decide label \"${a[@]}\"",
            HUMAN_ONLY,
        ),
    ]);
}

#[test]
fn a_model_actor_or_no_actor_beside_an_unread_call_stays_allowed() {
    assert_table(&[
        ("anneal decide label \"$@\"", ALLOW),
        ("anneal decide label \"$id\" yes", ALLOW),
        ("anneal decide label d1 yes --actor model", ALLOW),
        ("anneal decide label $ID yes --actor model", ALLOW),
        (
            "set -- d1 yes --actor model; anneal decide label \"$@\"",
            ALLOW,
        ),
        (
            "set -- d1 yes --actor model; anneal decide label \"${x:-$@}\"",
            ALLOW,
        ),
    ]);
}

#[test]
fn operands_known_only_at_run_time_stay_allowed_without_a_human_actor() {
    assert_table(&[
        ("anneal decide label \"$@\"", ALLOW),
        ("anneal decide label \"${args[@]}\"", ALLOW),
        ("anneal decide label $ARGS", ALLOW),
        ("anneal decide label \"$id\" yes", ALLOW),
        ("anneal decide label \"$id\" yes --actor model", ALLOW),
        ("anneal decide label \"$*\" \"${args[*]}\"", ALLOW),
        (
            "v() { anneal decide label \"$@\"; }; v d1 yes --actor model",
            ALLOW,
        ),
    ]);
}

#[test]
fn an_unread_anneal_path_without_a_literal_human_actor_stays_allowed() {
    assert_table(&[
        (
            "bash -c 'anneal \"$@\"' _ verdict set --pr 7 --actor model",
            ALLOW,
        ),
        (
            "bash -c 'anneal $0 \"$@\"' verdict set --actor humans",
            ALLOW,
        ),
        ("set -- plan check docs/plans/x.md; anneal \"$@\"", ALLOW),
        (
            "args=(verdict set --pr 7 --actor model); anneal \"${args[@]}\"",
            ALLOW,
        ),
        (
            "set -- verdict set --pr 7 --actor human; echo \"$@\"",
            ALLOW,
        ),
        ("echo --actor human; anneal verdict valid --pr 7", ALLOW),
    ]);
}
