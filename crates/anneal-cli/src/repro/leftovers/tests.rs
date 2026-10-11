//! The `[DEBUG-<id>]` tag matcher, and `debug_leftovers` on scratch git repositories.
//!
//! Tags are built with [`tag`] rather than written out, so this file holds no leftover itself.

use std::path::Path;
use std::process::Command;

use super::{LIMITS, Leftover, Limits, MAX_FILE_BYTES, debug_leftovers, scan, tagged};

/// A debug tag with `id`, built so the source never spells one.
fn tag(id: &str) -> String {
    format!("[{}-{id}]", "DEBUG")
}

/// Runs `git <args>` in `dir`, isolated from the caller's repository, and panics if it fails.
fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(["-c", "init.defaultBranch=main"])
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A scratch repository with `files` written (relative path, content) and `tracked` added.
fn repo(files: &[(&str, &str)], tracked: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temp dir");
    git(dir.path(), &["init", "-q"]);
    for (rel, content) in files {
        let path = dir.path().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create the parent");
        }
        std::fs::write(path, content).expect("write the file");
    }
    if !tracked.is_empty() {
        let mut args = vec!["add", "--"];
        args.extend_from_slice(tracked);
        git(dir.path(), &args);
    }
    dir
}

fn leftover(file: &str, line: usize) -> Leftover {
    Leftover {
        file: file.to_owned(),
        line,
    }
}

#[test]
fn a_tag_is_debug_then_an_id_then_a_closing_bracket() {
    let line = format!("eprintln!(\"{} x={{x}}\");", tag("a1"));

    assert!(tagged(&line), "a tag with an id: {line}");
    assert!(tagged(tag("parse_2-b")), "letters, digits, `_` and `-`");
    assert!(
        tagged(format!("[DEBUG-<id>] then {}", tag("z"))),
        "a real tag after a placeholder"
    );
}

#[test]
fn a_placeholder_an_empty_id_or_an_unclosed_tag_is_not_a_tag() {
    for line in [
        "tag instrumentation [DEBUG-<id>]",
        "[DEBUG-]",
        "[DEBUG-a1",
        "[debug-a1]",
        "DEBUG-a1]",
        "[DEBUG-a b]",
    ] {
        assert!(!tagged(line), "not a tag: {line}");
    }
    let long = format!("[{}-{}]", "DEBUG", "x".repeat(65));
    assert!(!tagged(&long), "an id is at most 64 characters");
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn a_tracked_files_tags_are_reported_with_their_file_and_line_in_order() {
    let source = format!(
        "fn a() {{}}\n// {}\nfn b() {{}}\nlet x = 1; // {}\n",
        tag("a1"),
        tag("a2")
    );
    let dir = repo(
        &[("src/lib.rs", &source), ("README.md", "clean\n")],
        &["src/lib.rs", "README.md"],
    );

    let found = debug_leftovers(dir.path()).expect("the scan runs");

    assert_eq!(
        found.leftovers,
        vec![leftover("src/lib.rs", 2), leftover("src/lib.rs", 4)],
        "both lines, by file then line"
    );
    assert_eq!((found.omitted, found.skipped.len()), (0, 0), "{found:?}");
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn untracked_files_are_read_and_ignored_files_are_not() {
    let line = format!("{}\n", tag("u1"));
    let dir = repo(
        &[
            (".gitignore", "target/\n"),
            ("notes/new.md", &line),
            ("target/out.txt", &line),
        ],
        &[".gitignore"],
    );

    let found = debug_leftovers(dir.path()).expect("the scan runs");

    assert_eq!(
        found.leftovers,
        vec![leftover("notes/new.md", 1)],
        "the untracked prose file, not the ignored one"
    );
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn a_file_over_the_cap_is_listed_as_skipped_not_read() {
    let big = format!(
        "{}\n{}",
        tag("big"),
        "x".repeat(usize::try_from(MAX_FILE_BYTES).expect("1 MiB fits"))
    );
    let dir = repo(&[("big.log", &big)], &[]);

    let found = debug_leftovers(dir.path()).expect("the scan runs");

    assert_eq!(found.leftovers, Vec::new(), "not read");
    assert_eq!(found.skipped, vec!["big.log".to_owned()], "listed");
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn a_clean_worktree_has_no_leftovers() {
    let dir = repo(&[("src/main.rs", "fn main() {}\n")], &["src/main.rs"]);

    let found = debug_leftovers(dir.path()).expect("the scan runs");

    assert!(found.is_clean(), "{found:?}");
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn past_1000_tagged_lines_the_rest_are_counted_as_omitted() {
    let lines: Vec<String> = (0..1005).map(|n| tag(&format!("n{n}"))).collect();
    let dir = repo(&[("many.txt", &lines.join("\n"))], &[]);

    let found = debug_leftovers(dir.path()).expect("the scan runs");

    assert_eq!(
        (found.leftovers().len(), found.omitted(), found.is_clean()),
        (1000, 5, false),
        "1,000 listed, 5 counted"
    );
    assert_eq!(
        found.leftovers().last(),
        Some(&leftover("many.txt", 1000)),
        "the first 1,000 in line order"
    );
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn past_the_total_byte_budget_files_are_skipped_and_the_scan_says_so() {
    let line = format!("{}\n", tag("b"));
    let dir = repo(&[("a.txt", &line), ("b.txt", &line)], &[]);
    let one_file = u64::try_from(line.len()).expect("a short line");
    let limits = Limits {
        total_bytes: one_file,
        ..LIMITS
    };

    let found = scan(dir.path(), limits).expect("the scan runs");

    assert_eq!(
        found.leftovers(),
        [leftover("a.txt", 1)],
        "the first file is read"
    );
    assert_eq!(found.skipped(), ["b.txt".to_owned()], "the second is not");
    assert!(found.over_budget(), "the scan reports it ran out of budget");
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn a_scan_within_the_default_budget_does_not_report_it_spent() {
    let dir = repo(&[("a.txt", "clean\n")], &["a.txt"]);

    let found = debug_leftovers(dir.path()).expect("the scan runs");

    assert!(!found.over_budget(), "{found:?}");
}

#[cfg(unix)]
#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn a_file_whose_name_is_not_utf8_is_still_read() {
    use std::os::unix::ffi::OsStrExt;

    let dir = repo(&[], &[]);
    let name = std::ffi::OsStr::from_bytes(b"bad\xff.txt");
    // APFS (macOS) refuses names that are not UTF-8; there is nothing to scan then.
    if std::fs::write(dir.path().join(name), format!("{}\n", tag("u"))).is_err() {
        return;
    }

    let found = debug_leftovers(dir.path()).expect("the scan runs");

    assert_eq!(
        found.leftovers(),
        [leftover("bad\u{FFFD}.txt", 1)],
        "read through its raw name, reported lossily"
    );
    assert_eq!(found.skipped(), Vec::<String>::new(), "not skipped");
}
