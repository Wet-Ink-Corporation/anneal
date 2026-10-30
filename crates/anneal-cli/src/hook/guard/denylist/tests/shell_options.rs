//! A shell's argument known only at run time may be options (#129): with `o=-c`,
//! `bash $o 'rm -rf /'` runs `rm -rf /`, as does `bash "$y"-c 'rm -rf /'` with `y` unset. Each
//! row was checked with bash 5 and dash using `echo` in place of `rm`.

use super::{ALLOW, RM_RF, UNREADABLE, assert_table};

#[test]
fn a_dynamic_shell_argument_that_may_be_c_has_the_next_word_read_as_its_script() {
    assert_table(&[
        ("bash $o 'rm -rf /'", RM_RF),
        ("bash \"$o\" 'rm -rf /'", RM_RF),
        ("bash ${o} 'rm -rf /'", RM_RF),
        ("bash \"$y\"-c 'rm -rf /'", RM_RF),
        ("bash -\"$y\"c 'rm -rf /'", RM_RF),
        ("bash \"$(printf -- -c)\" 'rm -rf /'", RM_RF),
        ("sh \"$o\" 'rm -rf /'", RM_RF),
        ("dash $o 'rm -rf /'", RM_RF),
        ("x='rm -rf /'; bash $o \"$x\"", RM_RF),
    ]);
}

#[test]
fn a_dynamic_shell_argument_may_be_any_options_before_the_script() {
    assert_table(&[
        // `o=-c`, then `-e`: the string is the first operand after every option.
        ("bash \"$o\" -e 'rm -rf /'", RM_RF),
        // `o=-e`: the options go on to the `-c`.
        ("bash \"$o\" -c 'rm -rf /'", RM_RF),
        ("bash -e \"$o\" 'rm -rf /'", RM_RF),
        // `o=-o`: it takes `pipefail` as its value.
        ("bash \"$o\" pipefail -c 'rm -rf /'", RM_RF),
        // `x=-e` or `x=--` after `-c`: the string is the next word.
        ("bash -c \"$x\" 'rm -rf /'", RM_RF),
        // An unquoted expansion that is empty vanishes (word splitting, #128); read as options,
        // it leads to the same script.
        ("bash -c $x 'rm -rf /'", RM_RF),
        ("caffeinate bash $x -c 'rm -rf /'", RM_RF),
        ("bash {-xc,$x} 'rm -rf /'", RM_RF),
        // `o=-s`, or any other option: the script is standard input.
        ("echo 'rm -rf /' | bash \"$o\"", RM_RF),
        ("echo 'rm -rf /' | bash \"$o\" x", RM_RF),
        // An option missing its value leaves no operand, as before #129.
        ("echo 'rm -rf /' | bash -o", RM_RF),
        ("echo 'rm -rf /' | bash \"$o\" -o", RM_RF),
    ]);
}

#[test]
fn a_plus_cluster_holding_c_or_s_is_read_as_its_minus_form() {
    assert_table(&[
        ("bash +c 'rm -rf /'", RM_RF),
        ("bash +ec 'rm -rf /'", RM_RF),
        ("sh +c 'rm -rf /'", RM_RF),
        ("bash +$x 'rm -rf /'", RM_RF),
        ("echo 'rm -rf /' | bash +s", RM_RF),
        ("bash +x ./run.sh", ALLOW),
        ("bash +O extglob ./run.sh", ALLOW),
    ]);
}

#[test]
fn wrappers_reach_the_same_shell_reader() {
    assert_table(&[
        ("env bash \"$o\" 'rm -rf /'", RM_RF),
        ("env -i bash $o 'rm -rf /'", RM_RF),
        ("sudo bash \"$o\" 'rm -rf /'", RM_RF),
        ("sudo -u root bash $o 'rm -rf /'", RM_RF),
        ("nohup bash \"$y\"-c 'rm -rf /'", RM_RF),
        ("caffeinate bash \"$o\" 'rm -rf /'", RM_RF),
        ("su root -- \"$o\" 'rm -rf /'", RM_RF),
        ("bash -c 'bash \"$o\" \"rm -rf /\"'", RM_RF),
    ]);
}

#[test]
fn a_script_the_next_word_cannot_be_read_as_is_denied() {
    assert_table(&[
        ("bash \"$o\" 'echo ('", UNREADABLE),
        // A cost: `bash "$o"` may read its script from stdin, here known only at run time.
        ("cat x | bash \"$script\"", UNREADABLE),
    ]);
}

#[test]
fn many_shell_arguments_that_may_be_options_are_read_in_linear_time() {
    // Each may be the `-c` string after the ones before it; a walk forking per word would
    // never end, and a list of readings compared pairwise would take minutes.
    let words = format!("bash{}", " \"$a\"".repeat(20_000));
    let last = format!("bash{} 'rm -rf /'", " \"$a\"".repeat(20_000));

    assert_table(&[(&words, ALLOW), (&last, RM_RF)]);
}

#[test]
fn a_dynamic_shell_argument_that_cannot_be_an_option_stays_a_script_path() {
    assert_table(&[
        ("bash \"$script\"", ALLOW),
        ("bash \"$script\" --verbose", ALLOW),
        ("bash \"$script\" build test", ALLOW),
        ("bash \"$dir\"/run.sh -c 'rm -rf /'", ALLOW),
        ("bash \"$s\".sh 'rm -rf /'", ALLOW),
        ("bash ./run.sh -c 'rm -rf /'", ALLOW),
        ("bash run.sh \"$o\" 'rm -rf /'", ALLOW),
        ("bash -- \"$o\" 'rm -rf /'", ALLOW),
        ("bash - \"$o\" 'rm -rf /'", ALLOW),
        ("bash -c \"$cmd\"", ALLOW),
        ("bash $o \"$cmd\"", ALLOW),
        ("sudo bash \"$HOME\"/bin/deploy.sh", ALLOW),
    ]);
}
