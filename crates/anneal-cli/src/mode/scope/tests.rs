use super::{PathKind, classify, normalise, relative, worktree_key};

#[test]
fn the_handoff_note_at_the_root_is_scratch_and_elsewhere_is_not() {
    assert_eq!(classify(".continue-here", &[]), PathKind::Scratch);
    assert_eq!(classify(".Continue-Here", &[]), PathKind::Scratch);
    assert_eq!(classify("src/.continue-here", &[]), PathKind::Source);
}

#[test]
fn a_posix_path_keeps_its_root_and_drops_dot_and_empty_components() {
    assert_eq!(normalise("/repo/./src//lib.rs"), "/repo/src/lib.rs");
}

#[test]
fn a_windows_path_reads_backslashes_as_slashes_and_keeps_its_drive() {
    assert_eq!(
        normalise(r"C:\Users\me\repo\src\lib.rs"),
        "C:/Users/me/repo/src/lib.rs"
    );
}

#[test]
fn dot_dot_is_taken_lexically_and_never_climbs_above_the_root_or_drive() {
    assert_eq!(normalise("/repo/tests/../src/lib.rs"), "/repo/src/lib.rs");
    assert_eq!(normalise("/../../etc/passwd"), "/etc/passwd");
    assert_eq!(normalise(r"C:\..\repo\a.rs"), "C:/repo/a.rs");
}

#[test]
fn a_relative_path_keeps_its_leading_dot_dots() {
    assert_eq!(normalise("../x/./y"), "../x/y");
    assert_eq!(normalise("a/../../b"), "../b");
}

#[test]
fn a_unc_path_keeps_its_double_slash() {
    assert_eq!(
        normalise(r"\\server\share\repo\a.rs"),
        "//server/share/repo/a.rs"
    );
}

#[test]
fn a_path_inside_the_root_is_made_relative_in_any_letter_case() {
    assert_eq!(relative("/repo", "/repo/src/lib.rs"), Some("src/lib.rs"));
    assert_eq!(
        relative("C:/Users/me/repo", "c:/users/ME/Repo/src/lib.rs"),
        Some("src/lib.rs")
    );
    assert_eq!(
        relative("/repo/", "/repo/a.rs"),
        Some("a.rs"),
        "a trailing /"
    );
}

#[test]
fn a_path_outside_the_root_or_a_sibling_with_the_same_prefix_is_not_relative() {
    assert_eq!(relative("/repo", "/other/a.rs"), None);
    assert_eq!(
        relative("/repo", "/repository/a.rs"),
        None,
        "not a prefix match"
    );
    assert_eq!(relative("/repo", "src/a.rs"), None, "a relative path");
}

#[test]
fn the_root_itself_is_the_empty_relative_path() {
    assert_eq!(relative("/repo", "/repo"), Some(""));
}

#[test]
fn every_path_under_the_root_of_a_filesystem_root_is_inside_it() {
    assert_eq!(relative("/", "/a/b.rs"), Some("a/b.rs"));
}

#[test]
fn source_files_are_source() {
    for path in ["src/main.rs", "crates/x/src/lib.rs", "Cargo.toml", "app.py"] {
        assert_eq!(classify(path, &[]), PathKind::Source, "{path}");
    }
}

#[test]
fn rust_test_module_files_are_tests() {
    for path in [
        "src/hook/guard/tests.rs",
        "src/parser/test.rs",
        "src/Tests.rs",
        "crates/x/src/tests/forms.rs",
    ] {
        assert_eq!(classify(path, &[]), PathKind::Test, "{path}");
    }
    for path in ["src/testsuite.rs", "src/contest.rs", "src/tests_helper.rs"] {
        assert_eq!(classify(path, &[]), PathKind::Source, "{path}");
    }
}

#[test]
fn the_worktree_key_reads_backslashes_as_slashes() {
    assert_eq!(
        worktree_key(r"C:\work\repo").replace("c:", "C:"),
        "C:/work/repo",
        "separators normalised"
    );
}

#[test]
#[cfg(any(windows, target_os = "macos"))]
fn the_worktree_key_ignores_letter_case_where_the_file_system_does() {
    assert_eq!(
        worktree_key("/Users/Me/Repo"),
        "/users/me/repo",
        "one key for every casing"
    );
}

#[test]
#[cfg(not(any(windows, target_os = "macos")))]
fn the_worktree_key_keeps_letter_case_where_the_file_system_does() {
    assert_eq!(
        worktree_key("/home/Me/Repo"),
        "/home/Me/Repo",
        "case-sensitive file systems keep distinct worktrees"
    );
}

#[test]
fn test_files_are_tests_in_any_letter_case() {
    for path in [
        "tests/cli.rs",
        "crates/x/tests/it/main.rs",
        "src/parser_test.go",
        "web/app.test.ts",
        "web/app.spec.js",
        "test_api.py",
        "Tests/Cli.rs",
    ] {
        assert_eq!(classify(path, &[]), PathKind::Test, "{path}");
    }
}

#[test]
fn scratch_directories_are_scratch_only_at_the_root() {
    for path in [
        ".temper/plans/x.rs",
        ".anneal/a",
        "tmp/x.rs",
        "scratch/y.rs",
        "TMP/z",
    ] {
        assert_eq!(classify(path, &[]), PathKind::Scratch, "{path}");
    }
    assert_eq!(
        classify("src/tmp/x.rs", &[]),
        PathKind::Source,
        "nested tmp"
    );
}

#[test]
fn prose_documents_are_docs() {
    for path in ["README.md", "docs/x.MD", "notes.txt", "docs/index.rst"] {
        assert_eq!(classify(path, &[]), PathKind::Doc, "{path}");
    }
    assert_eq!(
        classify("src/md.rs", &[]),
        PathKind::Source,
        "md in the name"
    );
}

#[test]
fn bytecode_tool_caches_and_logs_are_generated_at_any_depth() {
    for path in [
        "src/app/__pycache__/__init__.cpython-313.pyc",
        "tests/__pycache__/test_cli.cpython-313.pyc",
        "legacy.pyc",
        "src/old.PYO",
        ".pytest_cache/v/cache/lastfailed",
        "pkg/.mypy_cache/3.13/app.data.json",
        ".ruff_cache/0.6.0/123",
        ".npm/_logs/2026-10-30T21_27_55_045Z-debug-0.log",
        "sample.log",
        "logs/App.LOG",
    ] {
        assert_eq!(classify(path, &[]), PathKind::Generated, "{path}");
    }
}

#[test]
fn authored_dot_directories_and_cache_names_inside_longer_names_stay_source() {
    for path in [
        ".github/workflows/ci.yml",
        ".cargo/config.toml",
        ".venv/lib/python3.13/site-packages/sitecustomize.py",
        "src/my__pycache__/x.py",
        "src/__pycache__.py",
        "src/catalog.rs",
        "src/log.rs",
        "src/blog/post.py",
    ] {
        assert_eq!(classify(path, &[]), PathKind::Source, "{path}");
    }
}

#[test]
fn code_and_config_written_into_a_cache_directory_stay_source() {
    for path in [
        "src/.cache/helpers.py",
        "__pycache__/x.js",
        ".npm/hook.SH",
        ".pytest_cache/conftest.toml",
        "pkg/.ruff_cache/settings.yml",
    ] {
        assert_eq!(classify(path, &[]), PathKind::Source, "{path}");
    }
}

#[test]
fn the_define_artifacts_are_artifacts_in_any_letter_case() {
    let artifacts = ["design-tree.json", "docs/spec.yaml"];
    assert_eq!(classify("Design-Tree.json", &artifacts), PathKind::Artifact);
    assert_eq!(classify("docs/spec.yaml", &artifacts), PathKind::Artifact);
    assert_eq!(
        classify("design-tree.json.bak", &artifacts),
        PathKind::Source
    );
}
