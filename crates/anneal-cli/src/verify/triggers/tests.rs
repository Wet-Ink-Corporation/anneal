//! D5's deterministic triggers: auth, payments and migrations paths, more than 400 changed
//! lines, and an untracked file too large to measure.

use super::{LINE_TRIGGER, Trigger, judge, rules};
use crate::floor_guard::change::Touched;

fn file(path: &str, lines: u64) -> Touched {
    Touched {
        path: path.to_owned(),
        old_path: None,
        lines: Some(lines),
    }
}

#[test]
fn auth_payments_and_migrations_paths_fire_with_their_category() {
    let rules = rules().expect("the default globs compile");
    let files = [
        file("src/auth/session.rs", 1),
        file("billing/invoice.py", 1),
        file("db/migrations/0001_init.sql", 1),
        file("src/parser.rs", 1),
    ];

    let triggers = judge(&files, &rules);

    let categories: Vec<(&str, &str)> = triggers
        .fired
        .iter()
        .filter_map(|trigger| match trigger {
            Trigger::Path { path, category, .. } => Some((path.as_str(), *category)),
            Trigger::Lines { .. } | Trigger::Unmeasured { .. } => None,
        })
        .collect();
    assert_eq!(
        categories,
        vec![
            ("src/auth/session.rs", "auth"),
            ("billing/invoice.py", "payments"),
            ("db/migrations/0001_init.sql", "migrations"),
        ],
        "each sensitive file once, with the first glob's category; the parser fires nothing"
    );
}

#[test]
fn ci_and_infra_paths_are_not_d5_triggers() {
    let rules = rules().expect("the default globs compile");

    let triggers = judge(
        &[
            file(".github/workflows/ci.yml", 3),
            file("infra/main.tf", 3),
        ],
        &rules,
    );

    assert_eq!(
        triggers.fired,
        Vec::new(),
        "only auth, payments and migrations"
    );
}

#[test]
fn a_file_renamed_from_a_sensitive_path_fires() {
    let rules = rules().expect("the default globs compile");
    let renamed = Touched {
        path: "src/money.rs".to_owned(),
        old_path: Some("src/payments.rs".to_owned()),
        lines: Some(0),
    };

    let triggers = judge(&[renamed], &rules);

    assert_eq!(
        triggers.fired,
        vec![Trigger::Path {
            path: "src/payments.rs".to_owned(),
            glob: "**/*payment*",
            category: "payments",
        }]
    );
}

#[test]
fn more_than_400_changed_lines_fire_and_exactly_400_does_not() {
    let rules = rules().expect("the default globs compile");

    let at = judge(&[file("a.rs", 300), file("b.rs", 100)], &rules);
    let over = judge(&[file("a.rs", 300), file("b.rs", 101)], &rules);

    assert_eq!(LINE_TRIGGER, 400);
    assert_eq!(at.lines, 400);
    assert_eq!(at.fired, Vec::new(), "400 is not more than 400");
    assert_eq!(
        over.fired,
        vec![Trigger::Lines {
            lines: 401,
            limit: 400
        }]
    );
}

#[test]
fn a_file_too_large_to_measure_fires() {
    let rules = rules().expect("the default globs compile");
    let big = Touched {
        path: "big.txt".to_owned(),
        old_path: None,
        lines: None,
    };

    let triggers = judge(&[big], &rules);

    assert_eq!(
        triggers.fired,
        vec![Trigger::Unmeasured {
            path: "big.txt".to_owned()
        }],
        "its size in lines is unknown, so it is reviewed"
    );
}

#[test]
fn listed_files_are_capped_and_the_rest_counted() {
    let rules = rules().expect("the default globs compile");
    let files: Vec<Touched> = (0..70).map(|n| file(&format!("auth/{n}.rs"), 1)).collect();

    let triggers = judge(&files, &rules);

    assert_eq!(triggers.fired.len(), 64, "at most 64 listed");
    assert_eq!(triggers.omitted, 6, "the rest counted");
}
