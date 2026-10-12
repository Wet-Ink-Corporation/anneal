use super::{
    DEFAULT_SENSITIVE, FileChange, MAX_LISTED, NumstatError, Reason, judge, parse_numstat,
    sensitive_rules,
};
use crate::glob::Glob;

fn change(path: &str, lines: u64) -> FileChange {
    FileChange {
        path: path.to_owned(),
        lines: Some(lines),
    }
}

fn reasons(changes: &[FileChange]) -> Vec<Reason> {
    let rules = sensitive_rules(Vec::new()).expect("the defaults compile");
    judge(changes, &rules).reasons
}

#[test]
fn two_files_may_skip_but_three_need_a_full_review() {
    let two = [change("src/a.rs", 1), change("src/b.rs", 1)];
    let three = [
        change("src/a.rs", 1),
        change("src/b.rs", 1),
        change("src/c.rs", 1),
    ];

    assert_eq!(
        (reasons(&two), reasons(&three)),
        (vec![], vec![Reason::TooManyFiles { files: 3 }]),
        "2 files or fewer"
    );
}

#[test]
fn forty_nine_lines_may_skip_but_fifty_need_a_full_review() {
    let forty_nine = [change("src/a.rs", 40), change("src/b.rs", 9)];
    let fifty = [change("src/a.rs", 40), change("src/b.rs", 10)];

    assert_eq!(
        (reasons(&forty_nine), reasons(&fifty)),
        (vec![], vec![Reason::TooManyLines { lines: 50 }]),
        "under 50 changed lines, added and deleted together"
    );
}

#[test]
fn a_sensitive_path_needs_a_full_review_however_small() {
    let auth = [change("src/auth/session.rs", 1)];

    assert_eq!(
        reasons(&auth),
        vec![Reason::Sensitive {
            path: "src/auth/session.rs".to_owned(),
            glob: "**/*auth*/**".to_owned(),
            category: "auth",
        }],
        "the first glob that matches is named"
    );
}

#[test]
fn each_default_category_catches_a_typical_path() {
    let rules = sensitive_rules(Vec::new()).expect("the defaults compile");
    let typical = [
        ("auth", "crates/api/src/oauth.rs"),
        ("payments", "app/billing/invoice.py"),
        ("migrations", "db/migrations/0042_add_index.sql"),
        ("ci", ".github/workflows/ci.yml"),
        ("infra", "deploy/terraform/main.tf"),
        ("gate config", ".anneal/config.json"),
    ];

    for (category, path) in typical {
        let decision = judge(&[change(path, 1)], &rules);
        let named: Vec<&str> = decision
            .reasons
            .iter()
            .filter_map(|reason| match reason {
                Reason::Sensitive { category, .. } => Some(*category),
                _ => None,
            })
            .collect();
        assert_eq!(named, [category], "{path}");
    }
    assert!(
        DEFAULT_SENSITIVE.len() >= typical.len(),
        "one or more globs per category"
    );
}

#[test]
fn a_configured_glob_adds_to_the_defaults() {
    let configured = vec![Glob::new("ledger/**").expect("valid")];
    let rules = sensitive_rules(configured).expect("compiles");

    let decision = judge(&[change("ledger/posting.rs", 1)], &rules);

    assert_eq!(
        decision.reasons,
        vec![Reason::Sensitive {
            path: "ledger/posting.rs".to_owned(),
            glob: "ledger/**".to_owned(),
            category: "configured",
        }],
        "configured globs are checked after the defaults"
    );
}

#[test]
fn a_binary_file_needs_a_full_review() {
    let binary = [FileChange {
        path: "assets/logo.png".to_owned(),
        lines: None,
    }];

    assert_eq!(
        reasons(&binary),
        vec![Reason::Binary {
            path: "assets/logo.png".to_owned()
        }],
        "its size in lines is unknown"
    );
}

#[test]
fn an_empty_diff_may_skip() {
    let decision = judge(&[], &sensitive_rules(Vec::new()).expect("compiles"));

    assert!(decision.skip(), "nothing to review");
}

#[test]
fn listed_files_are_capped_and_the_rest_counted() {
    let many: Vec<FileChange> = (0..MAX_LISTED + 3)
        .map(|n| change(&format!("db/migrations/{n}.sql"), 1))
        .collect();

    let decision = judge(&many, &sensitive_rules(Vec::new()).expect("compiles"));

    let sensitive = decision
        .reasons
        .iter()
        .filter(|reason| matches!(reason, Reason::Sensitive { .. }))
        .count();
    assert_eq!((sensitive, decision.omitted), (MAX_LISTED, 3), "H-13");
}

#[test]
fn numstat_z_output_reads_lines_paths_and_binary_files() {
    let output = b"3\t2\tsrc/a b.rs\x00-\t-\tlogo.png\x000\t7\tgone.rs\x00";

    let changes = parse_numstat(output).expect("valid numstat");

    assert_eq!(
        changes,
        vec![
            change("src/a b.rs", 5),
            FileChange {
                path: "logo.png".to_owned(),
                lines: None
            },
            change("gone.rs", 7),
        ],
        "added plus deleted; `-` is binary"
    );
}

#[test]
fn a_malformed_numstat_record_is_an_error() {
    assert!(
        matches!(
            parse_numstat(b"x\t1\tfile\0"),
            Err(NumstatError::Malformed { .. })
        ),
        "never a guess"
    );
}
