use super::{
    PathError, StoreSource, anneal_home_from, git_failure, ledger_store, ledger_store_within,
    read_capped, wait_with_deadline,
};
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{Cursor, Write};
use std::path::Path;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// `git <args>` with every stream piped.
fn piped_git(args: &[&str]) -> Child {
    Command::new("git")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("git runs")
}

#[test]
fn read_capped_keeps_cap_bytes_and_consumes_the_rest() {
    let input: Vec<u8> = (0..=255_u8).cycle().take(1000).collect();
    let mut reader = Cursor::new(input.as_slice());

    let kept = read_capped(&mut reader, 300).expect("in-memory read");

    assert_eq!(
        kept.as_slice(),
        input.get(..300).expect("1000 > 300"),
        "first cap bytes"
    );
    assert_eq!(reader.position(), 1000, "the reader was drained to the end");
}

#[test]
fn read_capped_keeps_everything_under_the_cap() {
    let kept = read_capped(Cursor::new(b"short".as_slice()), 300).expect("in-memory read");
    assert_eq!(kept, b"short", "nothing dropped under the cap");
}

#[test]
#[cfg_attr(miri, ignore = "spawns git")]
fn a_child_writing_more_than_a_pipe_buffer_finishes_in_time() {
    // 256 KiB of lines through `git stripspace`, which echoes them: far past a 64 KiB pipe.
    let input = b"line of output\n".repeat(256 * 1024 / 15);
    let expected_len = input.len();
    let mut child = piped_git(&["stripspace"]);
    let mut stdin = child.stdin.take().expect("stdin is piped");

    let output = std::thread::scope(|scope| {
        let writer = std::thread::Builder::new()
            .name("git-stdin".to_owned())
            .spawn_scoped(scope, move || stdin.write_all(&input))
            .expect("spawn writer");
        let output = wait_with_deadline(&mut child, Duration::from_secs(60));
        writer.join().expect("writer thread").expect("write to git");
        output
    })
    .expect("finishes instead of timing out");

    assert!(
        output.status.success(),
        "git succeeded: {:?}",
        output.status
    );
    assert_eq!(output.stdout.len(), expected_len, "every byte came back");
}

#[test]
#[cfg_attr(miri, ignore = "spawns git")]
fn a_child_past_its_deadline_is_killed_and_reaped() {
    // `git hash-object --stdin` waits for stdin to close; the test keeps it open.
    let mut child = piped_git(&["hash-object", "--stdin"]);

    let result = wait_with_deadline(&mut child, Duration::from_millis(50));

    assert!(
        matches!(
            result,
            Err(PathError::GitTimeout { timeout }) if timeout == Duration::from_millis(50)
        ),
        "got {result:?}"
    );
    assert!(
        child.try_wait().expect("status is available").is_some(),
        "the killed child was reaped"
    );
}

#[test]
#[cfg_attr(miri, ignore = "spawns git")]
fn a_child_that_finishes_in_time_returns_its_output() {
    let mut child = piped_git(&["hash-object", "--stdin"]);
    let mut stdin = child.stdin.take().expect("stdin is piped");
    stdin.write_all(b"hello\n").expect("write to git");
    drop(stdin);

    let output = wait_with_deadline(&mut child, Duration::from_secs(30)).expect("finishes");

    assert!(output.status.success(), "git succeeded: {output:?}");
    assert_eq!(
        output.stdout, b"ce013625030ba8dba906f756967f9e9ca394464a\n",
        "the blob id of \"hello\\n\""
    );
}

fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + use<> {
    let map: HashMap<String, OsString> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), OsString::from(v)))
        .collect();
    move |key| map.get(key).cloned()
}

fn absolute(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

#[test]
fn anneal_home_override_wins() {
    let over = absolute("over");
    let home = absolute("home");
    let lookup = env(&[
        ("ANNEAL_HOME", over.to_str().expect("UTF-8")),
        ("HOME", home.to_str().expect("UTF-8")),
    ]);
    assert_eq!(
        anneal_home_from(lookup).expect("resolves"),
        over,
        "override"
    );
}

#[test]
fn home_then_userprofile_get_dot_anneal() {
    let home = absolute("home");
    let profile = absolute("profile");
    let both = env(&[
        ("ANNEAL_HOME", ""),
        ("HOME", home.to_str().expect("UTF-8")),
        ("USERPROFILE", profile.to_str().expect("UTF-8")),
    ]);
    let windows = env(&[("USERPROFILE", profile.to_str().expect("UTF-8"))]);

    assert_eq!(
        anneal_home_from(both).expect("resolves"),
        home.join(".anneal"),
        "HOME first"
    );
    assert_eq!(
        anneal_home_from(windows).expect("resolves"),
        profile.join(".anneal"),
        "USERPROFILE when HOME is unset"
    );
}

#[test]
fn no_home_and_relative_home_are_errors() {
    let none = anneal_home_from(env(&[])).expect_err("nothing set");
    let relative = anneal_home_from(env(&[("ANNEAL_HOME", "rel/dir")])).expect_err("relative");

    assert!(matches!(none, PathError::NoHome), "got {none:?}");
    assert!(
        matches!(
            relative,
            PathError::RelativeHome {
                var: "ANNEAL_HOME",
                ..
            }
        ),
        "got {relative:?}"
    );
}

#[test]
fn the_store_flag_wins_over_the_environment() {
    let cwd = absolute("work");
    let lookup = env(&[("ANNEAL_STORE", "from-env.sqlite3")]);

    let found = ledger_store(Some(Path::new("flag.sqlite3")), lookup, &cwd).expect("resolves");

    assert_eq!(found.path, cwd.join("flag.sqlite3"), "relative to cwd");
    assert_eq!(found.source, StoreSource::Flag, "the flag chose it");
}

#[test]
fn anneal_store_is_used_relative_to_the_working_directory() {
    let cwd = absolute("work");

    let found = ledger_store(None, env(&[("ANNEAL_STORE", "l.sqlite3")]), &cwd).expect("resolves");

    assert_eq!(found.path, cwd.join("l.sqlite3"), "joined to cwd");
    assert_eq!(found.source, StoreSource::Env, "the variable chose it");
}

#[test]
#[cfg_attr(miri, ignore = "runs git in a subprocess")]
fn outside_git_an_empty_anneal_store_falls_through_to_the_folder() {
    let dir = tempfile::tempdir().expect("temp dir");
    // A `.git` file pointing nowhere stops git here, even inside an enclosing repository.
    std::fs::write(dir.path().join(".git"), "gitdir: does-not-exist\n").expect("write");

    let found = ledger_store(None, env(&[("ANNEAL_STORE", "")]), dir.path()).expect("resolves");

    assert_eq!(
        found.path,
        dir.path().join(".anneal/ledger.sqlite3"),
        "<folder>/.anneal"
    );
    assert_eq!(found.source, StoreSource::Folder, "outside git");
}

#[test]
#[cfg_attr(miri, ignore = "runs git in a subprocess")]
fn the_callers_git_timeout_bounds_store_resolution() {
    let dir = tempfile::tempdir().expect("temp dir");

    // The deadline is checked before git's exit is polled, so a zero timeout always expires.
    let result = ledger_store_within(None, env(&[]), dir.path(), Duration::ZERO);

    assert!(
        matches!(&result, Err(PathError::GitTimeout { timeout }) if timeout.is_zero()),
        "the caller's timeout, not GIT_TIMEOUT: {result:?}"
    );
}

#[test]
#[cfg_attr(miri, ignore = "runs git in a subprocess")]
fn a_git_failure_other_than_no_repository_is_an_error_not_the_folder() {
    let dir = tempfile::tempdir().expect("temp dir");
    // A `.git` file git cannot parse: git fails here, but not with "not a git repository".
    std::fs::write(dir.path().join(".git"), "not a gitdir line\n").expect("write");

    let result = ledger_store(None, env(&[]), dir.path());

    assert!(
        matches!(&result, Err(PathError::GitFailed { stderr, .. }) if stderr.contains("invalid gitfile format")),
        "got {result:?}"
    );
}

#[test]
fn only_not_a_git_repository_classifies_as_outside_git() {
    let dir = Path::new("/work");
    let outside = [
        "fatal: not a git repository (or any of the parent directories): .git\n",
        "fatal: not a git repository: /work/does-not-exist\n",
        // git 2.56 and later, for the same `.git` file pointing nowhere.
        "fatal: gitfile does not point to a valid repository: /work/.git\n",
    ];
    let failures = [
        "fatal: detected dubious ownership in repository at '/work'\n",
        "fatal: invalid gitfile format: /work/.git\n",
        "",
    ];

    for stderr in outside {
        let error = git_failure(dir, stderr.as_bytes());
        assert!(
            matches!(&error, PathError::NotARepository { dir: found, stderr: said } if found == dir && said == stderr.trim()),
            "{stderr:?} gave {error:?}"
        );
    }
    for stderr in failures {
        let error = git_failure(dir, stderr.as_bytes());
        assert!(
            matches!(&error, PathError::GitFailed { dir: found, stderr: said } if found == dir && said == stderr.trim()),
            "{stderr:?} gave {error:?}"
        );
    }
}
