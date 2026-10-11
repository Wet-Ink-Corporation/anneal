use super::{Category, Detector, FloorFinding, PostImage};
use crate::floor_guard::diff::{FileDiff, parse};

/// Runs the detector over `diff`, with each file's post-image taken from `post_images`
/// (`(path, content)`), and returns `(category, file, line)` of each finding.
fn detect(diff: &str, post_images: &[(&str, &str)]) -> Vec<(Category, String, Option<u64>)> {
    let mut detector = Detector::default();
    for file in parse(diff) {
        let post = post_images
            .iter()
            .find(|(path, _)| *path == file.path.as_ref())
            .map(|(_, content)| PostImage::of(content));
        detector.file(&file, post.as_ref());
    }
    summary(&detector.finish().findings)
}

fn summary(findings: &[FloorFinding]) -> Vec<(Category, String, Option<u64>)> {
    findings
        .iter()
        .map(|finding| (finding.category, finding.file.clone(), finding.line))
        .collect()
}

/// A diff of `path` adding `lines` at line 1.
fn adding(path: &str, lines: &[&str]) -> String {
    let mut diff = format!(
        "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -0,0 +1,{} @@\n",
        lines.len()
    );
    for line in lines {
        diff.push('+');
        diff.push_str(line);
        diff.push('\n');
    }
    diff
}

/// A diff of `path` replacing line 5 `old` with `new`.
fn replacing(path: &str, old: &str, new: &str) -> String {
    format!(
        "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -5 +5 @@\n-{old}\n+{new}\n"
    )
}

fn hit(category: Category, file: &str, line: u64) -> (Category, String, Option<u64>) {
    (category, file.to_owned(), Some(line))
}

#[test]
fn each_listed_suppression_on_an_added_line_is_a_finding() {
    let cases = [
        ("src/lib.rs", "#[allow(dead_code)]"),
        ("src/lib.rs", "#![allow(clippy::all)]"),
        ("src/lib.rs", "    #[ignore]"),
        ("src/lib.rs", "#[ignore = \"slow\"]"),
        ("web/a.ts", "// eslint-disable-next-line no-console"),
        ("web/a.js", "/* eslint-disable */"),
        ("app/x.py", "import os  # noqa: F401"),
        ("app/x.py", "import os  # NOQA"),
        ("web/a.test.ts", "it.skip('works', () => {})"),
        ("web/a.test.ts", "describe.skip('suite', () => {})"),
        ("app/test_x.py", "@pytest.mark.skip(reason='later')"),
        ("web/a.test.js", "xit('works', () => {})"),
        ("web/a.test.js", "  xit(\"works\")"),
    ];
    for (path, line) in cases {
        assert_eq!(
            detect(&adding(path, &[line]), &[]),
            vec![hit(Category::Suppression, path, 1)],
            "{line:?} in {path}"
        );
    }
}

#[test]
fn expect_iterator_skip_exit_and_prose_are_not_suppressions() {
    let cases = [
        (
            "src/lib.rs",
            "#[expect(dead_code, reason = \"temper owns this\")]",
        ),
        ("src/lib.rs", "let rest = items.iter().skip(1);"),
        ("src/main.rs", "std::process::exit(2);"),
        ("app/x.py", "sys.exit(1)"),
        (
            "docs/floor-guard.md",
            "Flags `#[allow(` and `// eslint-disable`.",
        ),
        ("src/lib.rs", "// a comment about allow lists"),
    ];
    for (path, line) in cases {
        assert_eq!(
            detect(&adding(path, &[line]), &[]),
            Vec::new(),
            "{line:?} in {path}"
        );
    }
}

#[test]
fn a_suppression_on_a_removed_line_is_not_a_finding() {
    let diff = replacing("src/lib.rs", "#[allow(dead_code)]", "fn kept() {}");

    assert_eq!(detect(&diff, &[]), Vec::new());
}

#[test]
fn a_deleted_test_file_is_a_finding_for_each_test_path_form() {
    for path in [
        "tests/it/cli.rs",
        "crates/x/tests/a.rs",
        "pkg/store_test.go",
        "web/a.test.ts",
        "web/a.spec.js",
        "app/test_models.py",
    ] {
        let diff = format!(
            "diff --git a/{path} b/{path}\ndeleted file mode 100644\n--- a/{path}\n+++ /dev/null\n@@ -1 +0,0 @@\n-x\n"
        );
        assert_eq!(
            detect(&diff, &[]),
            vec![(Category::DeletedTest, path.to_owned(), None)],
            "{path}"
        );
    }
}

#[test]
fn deleting_a_file_that_is_not_a_test_is_not_a_finding() {
    let path = "src/contest.rs";
    let diff = format!(
        "diff --git a/{path} b/{path}\ndeleted file mode 100644\n--- a/{path}\n+++ /dev/null\n@@ -1 +0,0 @@\n-x\n"
    );

    assert_eq!(detect(&diff, &[]), Vec::new());
}

#[test]
fn a_renamed_test_file_is_not_deleted() {
    let diff = "diff --git a/tests/a.rs b/tests/b.rs\nsimilarity index 100%\nrename from tests/a.rs\nrename to tests/b.rs\n";

    assert_eq!(detect(diff, &[]), Vec::new());
}

#[test]
fn a_test_file_renamed_out_of_a_test_path_is_a_deleted_test() {
    let diff = "diff --git a/tests/test_x.py b/old/x_py.bak\nsimilarity index 100%\nrename from tests/test_x.py\nrename to old/x_py.bak\n";

    assert_eq!(
        detect(diff, &[]),
        vec![(Category::DeletedTest, "tests/test_x.py".to_owned(), None)]
    );
}

#[test]
fn a_test_file_renamed_between_test_paths_is_not_deleted() {
    let diff = "diff --git a/tests/a.rs b/crates/x/tests/a.rs\nsimilarity index 100%\nrename from tests/a.rs\nrename to crates/x/tests/a.rs\n";

    assert_eq!(detect(diff, &[]), Vec::new());
}

#[test]
fn a_test_file_emptied_in_the_working_tree_is_a_finding() {
    let path = "web/a.test.ts";
    let diff = format!(
        "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1,2 +0,0 @@\n-import x from 'x'\n-const y = 1\n"
    );

    assert_eq!(
        detect(&diff, &[(path, "\n  \n")]),
        vec![(Category::EmptiedTest, path.to_owned(), None)]
    );
    assert_eq!(
        detect(&diff, &[(path, "it('still here', () => {})\n")]),
        Vec::new(),
        "a test file with content left is not emptied"
    );
}

#[test]
fn removing_more_tests_than_are_added_is_a_finding_per_language_marker() {
    let cases = [
        ("src/lib.rs", "#[test]"),
        ("src/lib.rs", "    #[tokio::test]"),
        ("web/a.test.ts", "  it('adds', () => {"),
        ("web/a.test.js", "test(\"adds\", () => {"),
        ("app/test_x.py", "    def test_adds(self):"),
        ("app/test_x.py", "async def test_adds():"),
    ];
    for (path, marker) in cases {
        let diff = format!(
            "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -7 +6,0 @@\n-{marker}\n"
        );
        assert_eq!(
            detect(&diff, &[(path, "kept\n")]),
            vec![hit(Category::RemovedTest, path, 7)],
            "{marker:?} in {path}"
        );
    }
}

#[test]
fn a_test_moved_between_files_is_not_a_removal() {
    let removed = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -7 +6,0 @@\n-#[test]\n";
    let added = adding("src/b.rs", &["#[test]"]);

    assert_eq!(
        detect(&format!("{removed}{added}"), &[("src/a.rs", "x\n")]),
        Vec::new(),
        "net zero tests across the diff"
    );
}

#[test]
fn a_test_marker_in_a_file_of_another_language_does_not_count() {
    let path = "src/lib.rs";
    let diff = format!(
        "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -7 +6,0 @@\n-    def test_x(self):\n"
    );

    assert_eq!(detect(&diff, &[(path, "x\n")]), Vec::new());
}

#[test]
fn a_lowered_coverage_floor_is_a_finding() {
    let cases = [
        ("pyproject.toml", "fail_under = 90", "fail_under = 80"),
        (".coveragerc", "fail-under: 90", "fail-under: 85.5"),
        (
            "ci.yml",
            "  run: pytest --cov-fail-under=90",
            "  run: pytest --cov-fail-under=70",
        ),
        (
            "ci.yml",
            "  run: coverage report --fail-under 90",
            "  run: coverage report --fail-under 10",
        ),
        ("cov.json", "\"min_coverage\": 80,", "\"min_coverage\": 79,"),
    ];
    for (path, old, new) in cases {
        assert_eq!(
            detect(&replacing(path, old, new), &[]),
            vec![hit(Category::CoverageLowered, path, 5)],
            "{old:?} -> {new:?}"
        );
    }
}

#[test]
fn coverage_threshold_children_count_only_under_coverage_threshold() {
    let diff = replacing("jest.config.js", "      lines: 90,", "      lines: 50,");

    assert_eq!(
        detect(
            &diff,
            &[(
                "jest.config.js",
                "module.exports = { coverageThreshold: { global: { lines: 50 } } }\n"
            )]
        ),
        vec![hit(Category::CoverageLowered, "jest.config.js", 5)]
    );
    assert_eq!(
        detect(
            &diff,
            &[("jest.config.js", "module.exports = { lines: 50 }\n")]
        ),
        Vec::new(),
        "`lines` elsewhere is not a coverage floor"
    );
}

#[test]
fn a_raised_coverage_floor_or_a_new_one_is_not_a_finding() {
    assert_eq!(
        detect(
            &replacing("pyproject.toml", "fail_under = 80", "fail_under = 90"),
            &[]
        ),
        Vec::new(),
        "raised"
    );
    assert_eq!(
        detect(&adding("pyproject.toml", &["fail_under = 10"]), &[]),
        Vec::new(),
        "new, with nothing to compare against"
    );
}

#[test]
fn a_raised_timeout_is_a_finding_and_a_lowered_one_is_not() {
    let cases = [
        (
            "ci.yml",
            "    timeout-minutes: 10",
            "    timeout-minutes: 60",
        ),
        (
            "jest.config.json",
            "  \"testTimeout\": 5000,",
            "  \"testTimeout\": 30000,",
        ),
        ("Cargo.toml", "timeout = 30", "timeout = 300"),
        ("cfg.toml", "timeout_ms = 100", "timeout_ms = 101"),
    ];
    for (path, old, new) in cases {
        assert_eq!(
            detect(&replacing(path, old, new), &[]),
            vec![hit(Category::TimeoutRaised, path, 5)],
            "{old:?} -> {new:?}"
        );
        assert_eq!(
            detect(&replacing(path, new, old), &[]),
            Vec::new(),
            "{new:?} -> {old:?} tightens"
        );
    }
}

#[test]
fn a_ratchet_value_moved_the_loose_way_is_a_finding() {
    let path = "quality/ratchet.json";
    assert_eq!(
        detect(
            &replacing(path, "  \"max_warnings\": 3,", "  \"max_warnings\": 4,"),
            &[]
        ),
        vec![hit(Category::RatchetLoosened, path, 5)],
        "max raised"
    );
    assert_eq!(
        detect(
            &replacing(path, "  \"minTests\": 120,", "  \"minTests\": 119,"),
            &[]
        ),
        vec![hit(Category::RatchetLoosened, path, 5)],
        "min lowered"
    );
    assert_eq!(
        detect(
            &replacing(path, "  \"max_warnings\": 3,", "  \"max_warnings\": 2,"),
            &[]
        ),
        Vec::new(),
        "max lowered tightens"
    );
    assert_eq!(
        detect(
            &replacing(
                "quality/other.json",
                "  \"max_warnings\": 3,",
                "  \"max_warnings\": 4,"
            ),
            &[]
        ),
        Vec::new(),
        "only *ratchet*.json files"
    );
}

#[test]
fn the_generic_ratchet_rule_leaves_the_anneal_ratchet_file_to_its_reader() {
    assert_eq!(
        detect(
            &replacing(
                ".anneal/ratchets.json",
                "  \"max_warnings\": 3,",
                "  \"max_warnings\": 4,"
            ),
            &[]
        ),
        Vec::new(),
        "`.anneal/ratchets.json` is judged by its schema, not by key names"
    );
}

#[test]
fn findings_are_bounded_while_collected_and_keep_the_first_in_order() {
    let lines: Vec<String> = (0..1_005).map(|_| "#[allow(x)]".to_owned()).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let diff = format!(
        "{}{}",
        adding("b.rs", &refs),
        adding("a.rs", &["#[allow(y)]"])
    );

    let mut detector = Detector::default();
    for file in parse(&diff) {
        detector.file(&file, None);
        assert!(
            detector.findings.len() <= super::MAX_FINDINGS,
            "never more than the cap held: {}",
            detector.findings.len()
        );
    }
    let report = detector.finish();

    let last = report.findings.last().expect("findings");
    assert_eq!(
        (last.file.as_str(), last.line, report.omitted),
        ("b.rs", Some(999), 6),
        "the first 1,000 in (file, line, category) order are kept, the rest counted"
    );
}

#[test]
fn findings_are_sorted_by_file_then_line_and_bounded() {
    let lines: Vec<String> = (0..1_005).map(|_| "#[allow(x)]".to_owned()).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let diff = format!(
        "{}{}",
        adding("b.rs", &refs),
        adding("a.rs", &["#[allow(y)]"])
    );

    let mut detector = Detector::default();
    for file in parse(&diff) {
        detector.file(&file, None);
    }
    let report = detector.finish();

    assert_eq!(report.findings.len(), super::MAX_FINDINGS, "bounded");
    assert_eq!(report.omitted, 6, "the rest counted");
    let first = report.findings.first().expect("findings");
    assert_eq!(
        (first.file.as_str(), first.line),
        ("a.rs", Some(1)),
        "sorted"
    );
}

#[test]
fn an_untracked_file_is_checked_for_suppressions() {
    let file = FileDiff::untracked("web/new.test.ts", "it('a', () => {})\nxit('b', () => {})\n");
    let mut detector = Detector::default();

    detector.file(&file, None);

    assert_eq!(
        summary(&detector.finish().findings),
        vec![hit(Category::Suppression, "web/new.test.ts", 2)]
    );
}
