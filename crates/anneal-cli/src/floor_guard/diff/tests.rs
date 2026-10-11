use super::{FileDiff, Status, parse};

/// `(number, text)` of each line.
fn lines(lines: &[super::Line<'_>]) -> Vec<(u64, String)> {
    lines
        .iter()
        .map(|line| (line.number, line.text.to_owned()))
        .collect()
}

fn only<'a, 'd>(files: &'a [FileDiff<'d>]) -> &'a FileDiff<'d> {
    assert_eq!(files.len(), 1, "one file: {files:?}");
    files.first().expect("one file")
}

#[test]
fn a_modified_file_yields_its_hunks_with_line_numbers_on_each_side() {
    let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -3,2 +3 @@ fn a() {
-old three
-old four
+new three
@@ -10,0 +10,2 @@
+added ten
+added eleven
";
    let files = parse(diff);
    let file = only(&files);

    assert_eq!(
        (file.path.as_ref(), file.old_path.as_ref(), file.status),
        ("src/lib.rs", "src/lib.rs", Status::Modified)
    );
    assert_eq!(file.hunks.len(), 2, "two hunks");
    let first = file.hunks.first().expect("first hunk");
    assert_eq!(
        lines(&first.removed),
        vec![(3, "old three".to_owned()), (4, "old four".to_owned())]
    );
    assert_eq!(lines(&first.added), vec![(3, "new three".to_owned())]);
    let second = file.hunks.get(1).expect("second hunk");
    assert_eq!(
        lines(&second.added),
        vec![
            (10, "added ten".to_owned()),
            (11, "added eleven".to_owned())
        ]
    );
    assert!(second.removed.is_empty(), "nothing removed");
}

#[test]
fn removed_lines_that_look_like_headers_stay_in_their_hunk() {
    let diff = "\
diff --git a/notes.txt b/notes.txt
--- a/notes.txt
+++ b/notes.txt
@@ -1,2 +1 @@
--- a/not-a-header
-+++ b/not-a-header
+kept
\\ No newline at end of file
";
    let files = parse(diff);
    let file = only(&files);

    let hunk = file.hunks.first().expect("one hunk");
    assert_eq!(
        lines(&hunk.removed),
        vec![
            (1, "-- a/not-a-header".to_owned()),
            (2, "+++ b/not-a-header".to_owned())
        ]
    );
    assert_eq!(lines(&hunk.added), vec![(1, "kept".to_owned())]);
}

#[test]
fn added_deleted_and_renamed_files_have_their_status_and_paths() {
    let diff = "\
diff --git a/tests/old.rs b/tests/old.rs
deleted file mode 100644
index 1111111..0000000
--- a/tests/old.rs
+++ /dev/null
@@ -1 +0,0 @@
-#[test]
diff --git a/empty_test.py b/empty_test.py
deleted file mode 100644
index e69de29..0000000
diff --git a/src/new.rs b/src/new.rs
new file mode 100644
--- /dev/null
+++ b/src/new.rs
@@ -0,0 +1 @@
+fn new() {}
diff --git a/tests/a.rs b/tests/b.rs
similarity index 100%
rename from tests/a.rs
rename to tests/b.rs
";
    let files = parse(diff);

    let summary: Vec<(&str, &str, Status)> = files
        .iter()
        .map(|file| (file.old_path.as_ref(), file.path.as_ref(), file.status))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("tests/old.rs", "tests/old.rs", Status::Deleted),
            ("empty_test.py", "empty_test.py", Status::Deleted),
            ("src/new.rs", "src/new.rs", Status::Added),
            ("tests/a.rs", "tests/b.rs", Status::Renamed),
        ]
    );
}

#[test]
fn quoted_paths_are_unescaped() {
    let diff = "\
diff --git \"a/tests/tab\\there.rs\" \"b/tests/tab\\there.rs\"
deleted file mode 100644
diff --git a/tests/caf\u{e9}.rs b/tests/caf\u{e9}.rs
--- \"a/tests/caf\\303\\251.rs\"
+++ \"b/tests/caf\\303\\251.rs\"
@@ -1 +1 @@
-a
+b
";
    let files = parse(diff);

    let paths: Vec<&str> = files.iter().map(|file| file.path.as_ref()).collect();
    assert_eq!(paths, vec!["tests/tab\there.rs", "tests/caf\u{e9}.rs"]);
}

#[test]
fn a_path_with_spaces_is_read_from_the_header() {
    let diff = "\
diff --git a/my tests/x_test.go b/my tests/x_test.go
deleted file mode 100644
index e69de29..0000000
";
    let files = parse(diff);

    assert_eq!(only(&files).path.as_ref(), "my tests/x_test.go");
}

#[test]
fn the_tab_git_appends_to_a_side_path_with_a_space_is_dropped() {
    // diff.c appends a TAB to `---`/`+++` paths that contain a space.
    let diff = "\
diff --git a/my tests/x_test.py b/my tests/x_test.py
--- a/my tests/x_test.py\t
+++ b/my tests/x_test.py\t
@@ -3 +2,0 @@
-def test_a():
";
    let files = parse(diff);
    let file = only(&files);

    assert_eq!(
        (file.old_path.as_ref(), file.path.as_ref(), file.status),
        ("my tests/x_test.py", "my tests/x_test.py", Status::Modified)
    );
}

#[test]
fn an_untracked_file_is_one_hunk_of_added_lines() {
    let file = FileDiff::untracked("new.py", "import os  # noqa\n\nx = 1\n");

    assert_eq!(file.status, Status::Added);
    let hunk = file.hunks.first().expect("one hunk");
    assert_eq!(
        lines(&hunk.added),
        vec![
            (1, "import os  # noqa".to_owned()),
            (2, String::new()),
            (3, "x = 1".to_owned())
        ]
    );
}
