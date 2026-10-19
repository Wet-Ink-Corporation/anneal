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
        // The operands may be bash's own: `-` ends its options, `+x` and `+o NAME` are flags.
        ("su root -g root - <(curl x)", UNREADABLE),
        ("su root -w PATH +x <(curl x)", UNREADABLE),
        ("su root -w PATH +o pipefail <(curl x)", UNREADABLE),
        ("su root -g root +x -- -c 'rm -rf /'", RM_RF),
        ("su root -w PATH +x -- -c 'rm -rf /'", RM_RF),
        // bash runs `x` as a script file here, but the words after `--` are read on their own.
        ("su root -g root x -- -c 'rm -rf /'", RM_RF),
        ("su root -g root - -- -c 'rm -rf /'", RM_RF),
        // bash's own grammar: each `o` takes a word, long options take one dash, `+c` is `-c`.
        ("su root -- -ox pipefail -c 'rm -rf /'", RM_RF),
        ("su root -- -oo pipefail errexit -c 'rm -rf /'", RM_RF),
        ("su - root -- -Ox extglob -c 'rm -rf /'", RM_RF),
        ("su root -g root -- -init-file x -c 'rm -rf /'", RM_RF),
        ("su root -w PATH -- -rcfile x -c 'rm -rf /'", RM_RF),
        ("su root +c 'rm -rf /'", RM_RF),
        ("su root -g root +ox pipefail +c 'rm -rf /'", RM_RF),
        // su runs `-c VALUE`; a value bash reads as an option leaves the string to what follows.
        ("su -c -x root 'rm -rf /'", RM_RF),
        ("su -c +x root 'rm -rf /'", RM_RF),
        ("su -c -- root 'rm -rf /'", RM_RF),
        ("su -c -o root pipefail 'rm -rf /'", RM_RF),
        ("su - root -c 'ls' -- x", ALLOW),
        // su passes `-c` or `-f` first, so bash reads no long option after it.
        ("su -c -rcfile root 'rm -rf /'", RM_RF),
        ("su -c -posix root errexit 'rm -rf /'", RM_RF),
        ("su -c -verbose root pipefail 'rm -rf /'", RM_RF),
        ("su -f root -- -rcfile 'rm -rf /'", RM_RF),
        ("su --fast root -- -rcfile 'rm -rf /'", RM_RF),
        ("POSIXLY_CORRECT=1 su -f root -rcfile 'rm -rf /'", RM_RF),
        ("su -f root -- -help -c 'rm -rf /'", RM_RF),
        // The `-` and the user may follow `--`: getopt hands su its operands, then those words.
        ("su - -- root -c 'rm -rf /'", RM_RF),
        ("su -- - root -c 'rm -rf /'", RM_RF),
        ("su -m - -- root -c 'rm -rf /'", RM_RF),
        ("su - -- root -lc 'rm -rf /'", RM_RF),
        ("su - -- root -s -c 'rm -rf /'", RM_RF),
        ("su - -- root +x -c 'rm -rf /'", RM_RF),
        ("su -c -x - -- root 'rm -rf /'", RM_RF),
        ("POSIXLY_CORRECT=1 su -- - root -c 'rm -rf /'", RM_RF),
        ("curl x | su - -- root", UNREADABLE),
        ("su - -- root -c 'echo a'", ALLOW),
    ]);
}

#[test]
fn a_shell_reads_its_options_as_bash_does() {
    assert_table(&[
        ("bash -ox pipefail -c 'rm -rf /'", RM_RF),
        ("bash -rcfile x -c 'rm -rf /'", RM_RF),
        ("bash +c 'rm -rf /'", RM_RF),
        ("sudo bash +c 'rm -rf /'", RM_RF),
        // dash (`sh`) has no long options: `-posix` is a cluster whose `o` takes a word.
        ("sh -posix errexit -c 'rm -rf /'", RM_RF),
        ("dash -posix errexit -c 'rm -rf /'", RM_RF),
        ("curl x | sh -posix errexit", UNREADABLE),
        ("zsh -help -c 'rm -rf /'", RM_RF),
        // BusyBox's shell applets are ash, its `bash` included.
        ("busybox bash -posix errexit -c 'rm -rf /'", RM_RF),
        ("curl x | busybox bash -posix errexit", UNREADABLE),
        ("busybox /bin/bash -posix errexit -c 'rm -rf /'", RM_RF),
        ("curl x | busybox /bin/bash -posix errexit", UNREADABLE),
        ("busybox env bash -posix errexit -c 'rm -rf /'", RM_RF),
        ("bash -o pipefail -c 'make test'", ALLOW),
        ("bash --norc -c 'ls'", ALLOW),
        ("bash -help", ALLOW),
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

/// Issue #165: a glob or extglob in a shell's or wrapper's option slot may expand to an
/// option (`-[c]` to `-c` beside a file named `-c`), so the option it may be cannot be read.
#[test]
fn a_pattern_that_may_expand_to_an_option_in_an_option_slot_is_denied() {
    assert_table(&[
        ("bash -[c] 'rm -rf /'", UNREADABLE),
        ("bash -@(c) 'rm -rf /'", UNREADABLE),
        ("bash -o errexit -? 'rm -rf /'", UNREADABLE),
        ("bash @(--)login -c 'echo a'", UNREADABLE),
        ("sh -* 'rm -rf /'", UNREADABLE),
        ("su -@(c) 'rm -rf /'", UNREADABLE),
        ("su root -[c] 'rm -rf /'", UNREADABLE),
        ("script -@(c) 'rm -rf /' /dev/null", UNREADABLE),
        ("flock /tmp/l -@(c) 'rm -rf /'", UNREADABLE),
        ("env -@(S) 'rm -rf /'", UNREADABLE),
        ("sudo -@(s) 'rm -rf /'", UNREADABLE),
        ("sudo -u root -[s] 'rm -rf /'", UNREADABLE),
        // Read as an operand too, so the denial names what that reading runs.
        ("nice -[n] 5 rm -rf /", RM_RF),
        ("sudo -u @(root) -[s] 'rm -rf /'", UNREADABLE),
        ("timeout -[s] KILL 5 echo", UNREADABLE),
        ("xargs -[I] x echo", UNREADABLE),
        // A pattern where a wrapper reads an option that runs a string.
        ("bash @(-c) 'rm -rf /'", UNREADABLE),
        ("bash ?c 'rm -rf /'", UNREADABLE),
        ("env -S@(rm -rf ~)", UNREADABLE),
        ("env --split-string=@(x)", UNREADABLE),
        ("env -S'rm -rf /'*", UNREADABLE),
    ]);
}

/// Issue #165, review round 1: the option part is judged whatever value is attached to it, a
/// shell's `+` signs an option as `-` does, and su hands its words after `--` to the user's
/// shell.
#[test]
fn an_attached_value_a_plus_sign_or_su_s_dashes_do_not_hide_the_option() {
    assert_table(&[
        // Beside a directory named `-Srm -rf `, the trailing `/` matches it.
        ("env -[S]'rm -rf /'", UNREADABLE),
        ("env -[S]'rm -rf ${HOME}'", UNREADABLE),
        ("env -@(S)'rm -rf ${HOME}'", UNREADABLE),
        ("su -[c]'rm -rf ~'", UNREADABLE),
        ("su root -[c]'rm -rf ~'", UNREADABLE),
        ("script -[c]'rm -rf ~'", UNREADABLE),
        ("bash +[c] 'rm -rf /'", UNREADABLE),
        ("bash [+]c 'rm -rf /'", UNREADABLE),
        ("bash +@(c) 'rm -rf /'", UNREADABLE),
        ("sh +[c] 'rm -rf /'", UNREADABLE),
        ("bash -o errexit +[c] 'rm -rf /'", UNREADABLE),
        ("sudo bash +[c] 'rm -rf /'", UNREADABLE),
        ("su root +[c] 'rm -rf /'", UNREADABLE),
        ("su root -- -[c] 'rm -rf /'", UNREADABLE),
        ("su -- root -[c] 'rm -rf /'", UNREADABLE),
        ("su - -- root -@(c) 'rm -rf /'", UNREADABLE),
        ("su root -- +[c] 'rm -rf /'", UNREADABLE),
    ]);
}

/// Issue #165, review round 1: after a shell's command string come its operands (`$0` and the
/// rest), which it reads as no options.
#[test]
fn the_operands_after_a_shell_s_command_string_are_no_slots() {
    assert_table(&[
        ("sh -c 'echo \"$@\"' *", ALLOW),
        ("bash -c 'echo hi' *", ALLOW),
        ("bash -xc 'echo hi' x -*", ALLOW),
        ("bash -c -x 'echo hi' *", ALLOW),
        ("bash -co errexit 'echo hi' *", ALLOW),
    ]);
}

/// Issue #165, review round 2: a shell's slots are read as the shell reads its options, so
/// the value of `-o`, `-O` or `-rcfile` does not end them, and a script ends them.
#[test]
fn a_shell_s_slots_follow_its_own_option_reading() {
    assert_table(&[
        ("bash -c -o pipefail -[e] 'rm -rf /'", UNREADABLE),
        ("bash -co errexit -[e] 'rm -rf /'", UNREADABLE),
        ("sh -co errexit -[e] 'rm -rf /'", UNREADABLE),
        ("bash -c -O extglob -[e] 'rm -rf /'", UNREADABLE),
        ("bash +c -o errexit -[e] 'rm -rf /'", UNREADABLE),
        ("bash -c -o errexit * 'rm -rf /'", UNREADABLE),
        ("sudo bash -co errexit -[e] 'rm -rf /'", UNREADABLE),
        ("bash -rcfile /dev/null -[c] 'rm -rf /'", UNREADABLE),
        ("bash -e run.sh *", ALLOW),
        ("sh -e build.sh *", ALLOW),
        ("bash -x ./deploy.sh ?", ALLOW),
        ("watch -n1 ls -*", ALLOW),
    ]);
}

/// Issue #165, review round 3: a `*` may hold an option's sign, letter and the start of its
/// attached value at once, so what follows it does not clear it where options take a value.
#[test]
fn a_star_may_hold_an_option_and_its_attached_value() {
    assert_table(&[
        ("env *' rm -rf ~'", UNREADABLE),
        ("env *='rm -rf ~'", UNREADABLE),
        ("script *' rm -rf ~'", UNREADABLE),
        ("su *' rm -rf ~'", UNREADABLE),
        ("su root *' rm -rf ~'", UNREADABLE),
        ("sudo *' -s'", UNREADABLE),
        // `?` is one character: it may be only the sign.
        ("env ?' rm -rf ~'", ALLOW),
        // A shell's options carry no value in their word.
        ("bash *.sh", ALLOW),
        ("bash *' x'", ALLOW),
    ]);
}

/// Issue #165: a pattern after the `--` that ends a wrapper's options is its command, while a
/// `--` an option takes as its value ends nothing, so the word after it is still a slot.
#[test]
fn a_pattern_after_a_wrapper_s_double_dash_is_its_command_not_an_option() {
    assert_table(&[
        ("sudo -- *.sh", ALLOW),
        ("env -- *.sh", ALLOW),
        ("nice -- *.sh", ALLOW),
        ("sudo -u root -- *.sh", ALLOW),
        ("sudo -- -[s] 'rm -rf /'", ALLOW),
        ("sudo -[u] root -- ls", UNREADABLE),
        ("sudo -u -- ls", ALLOW),
        ("sudo -u -- -[s] 'rm -rf /'", UNREADABLE),
        ("sudo --user -- -[s] 'rm -rf /'", UNREADABLE),
        ("env -u -- -[S]'rm -rf ~'", UNREADABLE),
        ("env -C -- -[S]'rm -rf ~'", UNREADABLE),
        ("xargs -E -- -[s] 'rm -rf /'", UNREADABLE),
        ("xargs -d -- -[s] 'rm -rf /'", UNREADABLE),
        ("sudo --user=-- -[s] 'rm -rf /'", UNREADABLE),
        ("doas -- *.sh", ALLOW),
        ("stdbuf -- *.sh", ALLOW),
        ("xargs -- -[s] x", ALLOW),
        // A wrapper that takes operands after its options keeps the slot after them.
        ("timeout -- 5 -[s] x", UNREADABLE),
        ("chrt -o 0 -- -[s] x", UNREADABLE),
    ]);
}

/// Issue #165: a pattern that cannot start an option, or that stands after the slots.
#[test]
fn a_pattern_outside_an_option_slot_is_read_as_before() {
    assert_table(&[
        ("bash run.sh *", ALLOW),
        ("bash ./x-*.sh", ALLOW),
        ("bash *.sh", ALLOW),
        ("sudo rm -f *.o", ALLOW),
        ("sudo -u root ls -[a]", ALLOW),
        ("timeout 5 ls *", ALLOW),
        ("xargs -n1 ls *", ALLOW),
        ("find . -name *.rs -print", ALLOW),
        ("env -i ls *", ALLOW),
        ("sudo [ -f x ]", ALLOW),
        ("xargs -I{} echo {}", ALLOW),
        ("sudo -u ~root ls", ALLOW),
        ("nice [ -f x ]", ALLOW),
        ("bash [ab]*.sh", ALLOW),
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
