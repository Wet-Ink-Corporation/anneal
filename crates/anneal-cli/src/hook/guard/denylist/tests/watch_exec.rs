//! Issue #150: `watch -x` (`--exec`) runs its words as an argv, without a shell, so they are
//! read as words, their quoting kept, as any wrapper's command is. Where watch's options cannot
//! be read for certain (an option its table does not know, a word known only at run time) the
//! words are read that way too.

use super::{ALLOW, FORCE_PUSH, RM_RF, assert_table};

#[test]
fn watch_exec_runs_its_words_as_a_command() {
    assert_table(&[
        ("watch -x sh -c 'rm -rf /'", RM_RF),
        ("watch --exec sh -c 'rm -rf /'", RM_RF),
        ("watch --ex sh -c 'rm -rf /'", RM_RF),
        ("watch -x bash -c 'rm -rf /'", RM_RF),
        ("watch -x -- sh -c 'rm -rf /'", RM_RF),
        ("watch -x sudo sh -c 'rm -rf /'", RM_RF),
        ("watch -x env sh -c 'rm -rf /'", RM_RF),
        ("watch -x git push -f origin main", FORCE_PUSH),
        ("watch -x rm -rf '/'", RM_RF),
        ("/usr/bin/watch -x sh -c 'rm -rf /'", RM_RF),
        ("sudo watch -x sh -c 'rm -rf /'", RM_RF),
        ("watch -x watch -x sh -c 'rm -rf /'", RM_RF),
    ]);
}

#[test]
fn watch_exec_combined_with_other_options_runs_its_words_as_a_command() {
    assert_table(&[
        ("watch -tx sh -c 'rm -rf /'", RM_RF),
        ("watch -xt sh -c 'rm -rf /'", RM_RF),
        ("watch -xd sh -c 'rm -rf /'", RM_RF),
        ("watch -n 1 -x sh -c 'rm -rf /'", RM_RF),
        ("watch -n1 -x sh -c 'rm -rf /'", RM_RF),
        ("watch -x -n1 sh -c 'rm -rf /'", RM_RF),
        ("watch -xn1 sh -c 'rm -rf /'", RM_RF),
        ("watch -xn 1 sh -c 'rm -rf /'", RM_RF),
        ("watch --interval=1 -x sh -c 'rm -rf /'", RM_RF),
        ("watch --interval 1 --exec sh -c 'rm -rf /'", RM_RF),
        ("watch -n 1 -d -t -x sh -c 'rm -rf /'", RM_RF),
        ("watch -x -n 1 -- sh -c 'rm -rf /'", RM_RF),
    ]);
}

#[test]
fn watch_options_read_unsure_are_read_as_exec_too() {
    assert_table(&[
        // An option the table does not know (a later watch's): read as a flag, and the
        // words after it as an argv too.
        ("watch -X sh -c 'rm -rf /'", RM_RF),
        ("watch --unknown sh -c 'rm -rf /'", RM_RF),
        ("watch -zx sh -c 'rm -rf /'", RM_RF),
        // A word known only at run time may be `-x`.
        ("watch $o sh -c 'rm -rf /'", RM_RF),
        ("watch \"$o\" sh -c 'rm -rf /'", RM_RF),
        ("watch $(echo -x) sh -c 'rm -rf /'", RM_RF),
        ("watch -n 1 $o sh -c 'rm -rf /'", RM_RF),
        ("watch -n $n sh -c 'rm -rf /'", RM_RF),
        ("watch --interval=$n sh -c 'rm -rf /'", RM_RF),
        ("watch $o -n 1 sh -c 'rm -rf /'", RM_RF),
    ]);
}

#[test]
fn watch_exec_near_misses_stay_allowed() {
    assert_table(&[
        ("watch -x ls -la", ALLOW),
        ("watch -x sh -c ls", ALLOW),
        ("watch -x sh -c 'ls -la /'", ALLOW),
        ("watch -n 1 -x rm -f /tmp/x", ALLOW),
        ("watch -n 1 ls", ALLOW),
        ("watch $o ls -la", ALLOW),
        // Without `-x` watch's words are joined and run by `sh -c`: `sh -c rm -rf /` runs `rm`.
        ("watch sh -c 'ls -la'", ALLOW),
        // `-d` takes the rest of its cluster as its value (`d::`): `-dx` is no `-x`, and
        // `sh -c rm -rf /` runs `rm` alone.
        ("watch -dx sh -c 'rm -rf /'", ALLOW),
    ]);
}
