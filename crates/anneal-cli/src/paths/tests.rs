use super::{PathError, anneal_home_from, read_capped, wait_with_deadline};
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{Cursor, Write};
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
