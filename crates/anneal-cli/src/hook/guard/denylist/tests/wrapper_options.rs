//! Issue #121: wrapper options read as each wrapper's getopt reads them. The forms the
//! installed binaries take are in [`super::wrapper_probes`]; doas and the BSD `env` and `xargs`
//! letters here are from their sources.

use super::{ALLOW, RM_RF, UNREADABLE, assert_table};

#[test]
fn a_cluster_ending_in_a_value_letter_takes_the_next_word() {
    assert_table(&[
        ("sudo -iu echo rm -rf /", RM_RF),
        ("sudo -nu echo rm -rf /", RM_RF),
        ("doas -nu echo rm -rf /", RM_RF),
        ("env -iu echo rm -rf /", RM_RF),
        ("env -vu echo rm -rf /", RM_RF),
        ("ionice -tc echo rm -rf /", RM_RF),
        ("timeout -vs echo 5 rm -rf /", RM_RF),
        ("stdbuf -oL -e echo rm -rf /", RM_RF),
        ("xargs -tn echo rm -rf /", RM_RF),
        ("watch -tn echo rm -rf /", RM_RF),
        ("flock -nw echo /l rm -rf /", RM_RF),
        ("chrt -vT echo rm -rf /", RM_RF),
    ]);
}

#[test]
fn value_letters_of_every_implementation_take_a_value() {
    assert_table(&[
        // sudo: Linux refuses `-a` and `-c`, BSD builds take a value; `-R` is the chroot.
        ("sudo -R echo rm -rf /", RM_RF),
        ("sudo -c echo rm -rf /", RM_RF),
        ("sudo -a echo rm -rf /", RM_RF),
        // OpenBSD's doas.
        ("doas -a echo rm -rf /", RM_RF),
        // FreeBSD's env and xargs.
        ("env -P echo rm -rf /", RM_RF),
        ("env -L echo rm -rf /", RM_RF),
        ("env -U echo rm -rf /", RM_RF),
        ("xargs -J echo rm -rf /", RM_RF),
        ("xargs -S echo rm -rf /", RM_RF),
        ("xargs -R echo rm -rf /", RM_RF),
    ]);
}

#[test]
fn an_optional_letter_takes_only_the_rest_of_its_cluster() {
    assert_table(&[
        ("xargs -eE rm -rf /", RM_RF),
        ("xargs -eecho rm -rf /", RM_RF),
        ("xargs -iecho rm -rf /", RM_RF),
        ("xargs -lecho rm -rf /", RM_RF),
        ("watch -dn rm -rf /", RM_RF),
        ("watch -decho rm -rf /", RM_RF),
        ("script -techo -c 'rm -rf /' /dev/null", RM_RF),
        ("sudo -hecho rm -rf /", RM_RF),
        // `-t` takes `qc`: no `-c`, so script runs nothing (two files are refused).
        ("script -tqc x rm -rf /", ALLOW),
        // The optional letter alone takes nothing: the next word is the command.
        ("xargs -e echo rm -rf /", ALLOW),
        ("xargs --eof echo rm -rf /", ALLOW),
    ]);
}

#[test]
fn chrt_reads_a_priority_only_when_it_is_all_digits() {
    assert_table(&[
        ("chrt -o rm -rf /", RM_RF),
        ("chrt -b rm -rf /", RM_RF),
        ("chrt -i rm -rf /", RM_RF),
        ("chrt -d rm -rf /", RM_RF),
        ("chrt -o 0 -- rm -rf /", RM_RF),
        ("chrt -o -- 0 rm -rf /", RM_RF),
        ("chrt -o 0 echo rm -rf /", ALLOW),
        ("chrt -f 10 echo rm -rf /", ALLOW),
        // `-p`: the words after the priority run when the last is a pid of 0 or -1.
        ("chrt -p 10 4242", ALLOW),
        ("chrt -p 10 rm -rf / 0", RM_RF),
        ("chrt -op 0 rm -rf / -1", RM_RF),
        ("taskset -p 0x1 rm -rf / 0", RM_RF),
        ("taskset -cp 0 rm -rf / -0", RM_RF),
    ]);
}

#[test]
fn every_command_string_of_su_and_script_is_read() {
    assert_table(&[
        ("su -c ls -c 'rm -rf /'", RM_RF),
        ("su -c ls --session-command='rm -rf /'", RM_RF),
        ("su root -lc 'rm -rf /'", RM_RF),
        ("su -fc'rm -rf /'", RM_RF),
        ("script -qc ls -c 'rm -rf /' /dev/null", RM_RF),
        ("script /dev/null -qc 'rm -rf /'", RM_RF),
        ("su -c ls -c 'echo a'", ALLOW),
        ("script -qc ls /dev/null", ALLOW),
    ]);
}

#[test]
fn su_arguments_after_the_user_are_read_as_the_users_shell_reads_them() {
    assert_table(&[
        ("su root -- -c 'rm -rf /'", RM_RF),
        ("su -- root -c 'rm -rf /'", RM_RF),
        ("su - root -- -c 'rm -rf /'", RM_RF),
        ("su -l root -- -c 'rm -rf /'", RM_RF),
        ("su root --login -- -c 'rm -rf /'", RM_RF),
        ("su root -s /bin/bash -- -c 'rm -rf /'", RM_RF),
        ("su root -- -lc 'rm -rf /'", RM_RF),
        // With POSIXLY_CORRECT, su stops at the user and hands `-s -c ...` to the shell.
        ("POSIXLY_CORRECT=1 su root -s -c 'rm -rf /'", RM_RF),
        ("su root -- -c 'echo a'", ALLOW),
        // The shell reads its options to the end, then takes its command string.
        ("su root -- -c -x 'rm -rf /'", RM_RF),
        ("su root -- -c -o pipefail 'rm -rf /'", RM_RF),
        ("su root -- -co pipefail 'rm -rf /'", RM_RF),
        ("su root -- -c -- 'rm -rf /'", RM_RF),
        ("POSIXLY_CORRECT=1 su root -c -e 'rm -rf /'", RM_RF),
        ("su - root -s /bin/sh -c 'rm -rf /'", RM_RF),
        // An operand after su's value options is the shell's script file.
        ("su root -w PATH <(curl x)", UNREADABLE),
        ("su root -g root /dev/fd/3", UNREADABLE),
        ("su - root -s /bin/bash <(curl x)", UNREADABLE),
        ("su root -g root x", ALLOW),
    ]);
}

#[test]
fn each_command_string_is_read_on_its_own() {
    assert_table(&[
        ("su -c 'echo \\' -c 'rm -rf /'", RM_RF),
        ("script -c 'echo \\' -c 'rm -rf /'", RM_RF),
        ("su -c 'cat <<EOF' -c 'rm -rf /'", RM_RF),
        ("script -qc 'cat <<EOF' -c 'rm -rf /' /dev/null", RM_RF),
    ]);
}

#[test]
fn options_added_after_the_probed_versions_take_their_values() {
    assert_table(&[
        // sudo takes the word after `-h` as a host and runs the command on the local host.
        ("sudo -h echo rm -rf /", RM_RF),
        ("H=echo; sudo -h $H rm -rf /", RM_RF),
        // coreutils 9.5's `env -a`, procps-ng 4.0.6's `watch -s`, util-linux's `chrt -U`/`-X`.
        ("env -a echo rm -rf /", RM_RF),
        ("env --argv0 echo rm -rf /", RM_RF),
        ("watch -s echo rm -rf /", RM_RF),
        ("chrt -U echo rm -rf /", RM_RF),
        ("chrt --clamp-max echo rm -rf /", RM_RF),
        // FreeBSD's env reads `--C` as `-` then `-C`, which takes the next word.
        ("env --C echo rm -rf /", RM_RF),
        ("env --S 'rm -rf /'", RM_RF),
    ]);
}

#[test]
fn long_options_take_their_value_written_out_or_abbreviated() {
    assert_table(&[
        ("sudo --us echo rm -rf /", RM_RF),
        ("sudo --preserve-env echo rm -rf /", ALLOW),
        ("sudo --preserve-env=PATH rm -rf /", RM_RF),
        ("env --un echo rm -rf /", RM_RF),
        ("env --block-signal rm -rf /", RM_RF),
        ("xargs --max-a echo rm -rf /", RM_RF),
        ("timeout --sig echo 5 rm -rf /", RM_RF),
        ("nice --adj echo rm -rf /", RM_RF),
    ]);
}

#[test]
fn ordinary_wrapped_commands_stay_allowed() {
    assert_table(&[
        ("sudo -iu postgres psql -c 'select 1'", ALLOW),
        ("sudo -u root -- make install", ALLOW),
        ("env -i PATH=/bin make test", ALLOW),
        ("env - make test", ALLOW),
        ("xargs -n1 -P4 echo", ALLOW),
        ("watch -n 1 ls", ALLOW),
        ("chrt -o 0 cargo test", ALLOW),
        ("timeout -s KILL 5 cargo test", ALLOW),
        ("ionice -c3 nice -n5 cargo build", ALLOW),
        ("flock -w 5 /tmp/l cargo test", ALLOW),
    ]);
}
