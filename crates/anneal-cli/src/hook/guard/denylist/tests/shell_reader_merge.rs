//! The shell reader after #121 and #129 met: every reading either kept, crossed with the
//! other's. A shell's argument known only at run time may be `-c`, so the word after it may be
//! the command string, and one that is itself known only at run time cannot be read: an
//! accepted false-positive cost (#129 review).

use super::{RM_RF, UNREADABLE, assert_table};

#[test]
fn a_dynamic_argument_before_a_dynamic_string_is_unreadable() {
    assert_table(&[
        ("bash \"$script\" \"$1\"", UNREADABLE),
        ("sh \"$0\" \"$@\"", UNREADABLE),
        ("bash \"$script\" \"$(date)\"", UNREADABLE),
    ]);
}

#[test]
fn a_dynamic_argument_may_be_options_after_a_bash_long_option_or_behind_su() {
    assert_table(&[
        // bash's long option, then `o=-c`.
        ("bash --norc \"$o\" 'rm -rf /'", RM_RF),
        ("bash -rcfile x \"$o\" 'rm -rf /'", RM_RF),
        // su hands its shell the words after `--`, and the user's shell may be ash.
        ("su root -- \"$o\" 'rm -rf /'", RM_RF),
        ("busybox sh \"$o\" 'rm -rf /'", RM_RF),
        ("busybox bash -posix x \"$o\" 'rm -rf /'", RM_RF),
    ]);
}

#[test]
fn a_two_dash_long_option_is_also_read_after_a_short_one() {
    assert_table(&[
        // bash reads `--rcfile` as a cluster here; read as before #121, it takes `x`.
        ("bash -e --rcfile x -c 'rm -rf /'", RM_RF),
        ("bash -e --init-file x -c 'rm -rf /'", RM_RF),
    ]);
}

#[test]
fn every_word_after_su_s_dashes_is_also_read_as_the_shell_s() {
    assert_table(&[
        // #129 read su's words after `--` as the shell's whatever stands before them.
        ("su - -- -c 'rm -rf /'", RM_RF),
        ("su - -- <(curl x)", UNREADABLE),
        ("su - -- /dev/fd/3", UNREADABLE),
    ]);
}

#[test]
fn a_plus_cluster_is_read_both_ways_without_long_options_too() {
    assert_table(&[
        ("zsh +s <(curl x)", UNREADABLE),
        ("sh -e +s <(curl x)", UNREADABLE),
        ("su root -- +c 'rm -rf /'", RM_RF),
        ("su root -- -e +s <(curl x)", UNREADABLE),
    ]);
}
