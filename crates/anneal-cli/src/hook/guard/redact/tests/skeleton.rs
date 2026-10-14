//! The skeleton itself: what each kind of word becomes, wrappers read from the denylist's
//! table, nested bodies, and the signal D1 keeps.

use super::super::{REDACTED, UNPARSED, allow, redact};
use super::assert_cases;

#[test]
fn a_command_the_lexer_cannot_read_is_one_fixed_marker() {
    let too_deep = format!("{}x{}", "$(".repeat(40), ")".repeat(40));

    assert_cases(&[
        ("echo 'hunter2", UNPARSED),
        ("cat > ", UNPARSED),
        (&too_deep, UNPARSED),
    ]);
}

#[test]
fn programs_and_separators_are_kept_and_arguments_hidden() {
    assert_cases(&[
        (
            "ls -l /etc/passwd | grep root; echo hi && pwd",
            "ls -l <redacted> | grep <redacted>; echo <redacted>; pwd",
        ),
        ("a |& b", "a | b"),
        ("", ""),
        ("# only a comment", ""),
    ]);
}

#[test]
fn a_program_word_that_is_not_a_short_literal_name_is_hidden() {
    let long = "a".repeat(65);
    let longest = "a".repeat(64);

    assert_cases(&[
        ("\"$CMD\" x", "<redacted> <redacted>"),
        ("./my\\ tool x", "<redacted> <redacted>"),
        ("/usr/bin/python3.12 x", "python3.12 <redacted>"),
        ("c++ x", "c++ <redacted>"),
        (&long, REDACTED),
        (&longest, &longest),
        ("ls*", REDACTED),
    ]);
}

#[test]
fn assignments_before_the_program_keep_only_a_short_name() {
    let long = format!("{}=x run", "A".repeat(65));

    assert_cases(&[
        ("A=1 B=2 run", "A=<redacted> B=<redacted> run"),
        ("A=1", "A=<redacted>"),
        (&long, "<redacted>=<redacted> run"),
    ]);
}

#[test]
fn only_a_word_the_denylist_reads_as_an_assignment_shows_its_name() {
    assert_cases(&[
        ("find . -name hunter2=x", "find . -name <redacted>"),
        ("echo hunter2=x", "echo <redacted>"),
        ("bash -s hunter2=x", "bash -s <redacted>"),
        (
            "sudo -u root K=v run",
            "sudo -u <redacted> K=<redacted> run",
        ),
        // nohup takes no assignments: it runs `A=1`, so the rest are its arguments.
        (
            "nohup A=1 env B=2 run",
            "nohup <redacted> <redacted> <redacted> <redacted>",
        ),
    ]);
}

#[test]
fn long_options_keep_their_name_and_hide_their_value() {
    assert_cases(&[
        (
            "git push --force-with-lease=origin/main",
            "git push --force-with-lease=<redacted>",
        ),
        ("tool --dry-run --x.y_z", "tool --dry-run --x.y_z"),
        ("tool --a=b=c", "tool --a=<redacted>"),
        ("tool --pa$x", "tool <redacted>"),
        ("tool \"--$x\"", "tool <redacted>"),
        ("tool --pa:ss", "tool <redacted>"),
        ("git checkout -- file", "git checkout -- <redacted>"),
        ("cat -", "cat -"),
    ]);
}

#[test]
fn a_short_option_is_kept_only_when_it_cannot_carry_a_value() {
    assert_cases(&[
        ("rm -rf /", "rm -rf /"),
        ("git clean -fdx", "git clean -fdx"),
        ("git push -f origin main", "git push -f origin main"),
        ("docker system prune -af", "docker system prune -af"),
        ("rm -rfz /", "rm -<redacted> /"),
        ("mysql -psecret", "mysql -<redacted>"),
        ("tool -p", "tool -p"),
        ("tool -é", "tool -<redacted>"),
        ("tool -ab", "tool -<redacted>"),
        (
            "terraform destroy -auto-approve -var x=1",
            "terraform destroy -auto-approve -<redacted> <redacted>",
        ),
    ]);
}

#[test]
fn subcommands_matter_only_for_their_own_program() {
    assert_cases(&[
        ("git push origin main", "git push origin main"),
        ("echo push delete", "echo <redacted> <redacted>"),
        ("kubectl delete ns prod", "kubectl delete ns <redacted>"),
        ("docker volume rm data", "docker volume rm <redacted>"),
        ("npm unpublish pkg", "npm unpublish <redacted>"),
        ("systemctl stop nginx", "systemctl stop <redacted>"),
        ("\"$GIT\" push", "<redacted> <redacted>"),
    ]);
}

#[test]
fn sensitive_targets_are_kept_wherever_they_stand() {
    assert_cases(&[
        ("rm -rf ~/ ./* .. *", "rm -rf ~/ ./* .. *"),
        ("git reset --hard HEAD", "git reset --hard HEAD"),
        (
            "git push origin master origin/main",
            "git push origin master origin/main",
        ),
        (
            "dd if=/dev/urandom of=/dev/nvme0n1p2 bs=4M",
            "dd if=/dev/urandom of=/dev/nvme0n1p2 <redacted>",
        ),
        ("dd of=disk.img", "dd of=<redacted>"),
        ("mkfs.ext4 /dev/sdb1", "mkfs.ext4 /dev/sdb1"),
        ("wipefs /dev/sda-backup", "wipefs <redacted>"),
        ("echo \"$HOME\"", "echo <redacted>"),
    ]);
}

#[test]
fn redirections_keep_the_operator_and_hide_the_target() {
    assert_cases(&[
        ("echo hi > out.txt 2>&1", "echo <redacted> > <redacted>"),
        ("echo hi >> /dev/null", "echo <redacted> > /dev/null"),
        ("cat /dev/zero > /dev/sda", "cat /dev/zero > /dev/sda"),
        ("cat in > /dev/sda1", "cat <redacted> > /dev/sda1"),
        ("sort < in.txt", "sort < <redacted>"),
        ("cat <<'EOF'\nhunter2\nEOF", "cat <<< <redacted>"),
        ("> file", "> <redacted>"),
    ]);
}

/// Every wrapper of the denylist's table (`denylist::wrappers`) that runs the words after it,
/// with options and values as it takes them.
const INLINE_WRAPPERS: &[&str] = &[
    "sudo",
    "sudo -u root",
    "doas",
    "env",
    "env -u HOME",
    "env -i",
    "command",
    "nohup",
    "setsid",
    "unbuffer",
    "exec",
    "exec -a name",
    "time",
    "time -p",
    "nice",
    "nice -n 5",
    "stdbuf -o L",
    "ionice -c 3",
    "timeout 5",
    "timeout -s KILL 5",
    "taskset 0x1",
    "flock /tmp/lock",
    "xargs",
    "xargs -n 1",
    "nohup timeout 5 nice -n 1",
];

/// The wrappers of the denylist's table that may not run the words after them as a program
/// (PR 120 review round 10): `builtin` runs only a builtin, `busybox` only an applet, the word
/// after `function NAME` or `coproc` may be a name, and chrt's priority may be out of range.
const MAYBE_WRAPPERS: &[&str] = &["builtin", "busybox", "coproc", "function f", "chrt -f 10"];

/// Every wrapper of the table that runs a string as shell text, before that string.
const STRING_WRAPPERS: &[&str] = &[
    "bash -c",
    "sh -c",
    "zsh -c",
    "dash -c",
    "ksh -c",
    "mksh -c",
    "ash -c",
    "eval",
    "su -c",
    "su root -c",
    "script -c",
    "watch",
    "flock /tmp/lock -c",
    "sudo bash -c",
    "xargs sh -c",
    "nohup bash -c",
];

#[test]
fn every_denylist_wrapper_shows_the_program_it_runs_and_no_value() {
    let command = "env x='Bearer' dropdb production";
    let quoted = "\"env x='Bearer' dropdb production\"";
    let commands = INLINE_WRAPPERS
        .iter()
        .map(|wrapper| format!("{wrapper} {command}"))
        .chain(
            STRING_WRAPPERS
                .iter()
                .map(|wrapper| format!("{wrapper} {quoted}")),
        )
        .chain([format!("find . -exec {command} \\;")]);
    for command in commands {
        let redacted = redact(&command);

        assert!(
            redacted.contains("dropdb")
                && !redacted.contains("Bearer")
                && !redacted.contains("production"),
            "{command:?} became {redacted:?}"
        );
    }
}

#[test]
fn a_wrapper_that_may_not_run_what_follows_shows_neither_it_nor_its_values() {
    let command = "env x='Bearer' dropdb production";
    let commands = MAYBE_WRAPPERS
        .iter()
        .map(|wrapper| format!("{wrapper} {command}"))
        .chain([
            format!("trap \"{command}\" EXIT"),
            format!("alias a=\"{command}\""),
        ]);
    for command in commands {
        let redacted = redact(&command);

        assert!(
            !redacted.contains("dropdb")
                && !redacted.contains("Bearer")
                && !redacted.contains("production"),
            "{command:?} became {redacted:?}"
        );
    }
}

#[test]
fn wrapper_options_and_values_follow_the_option_rules() {
    assert_cases(&[
        (
            "sudo -u root env -u HOME A=1 timeout -s KILL 5 rm -rf /",
            "sudo -u <redacted> env -u <redacted> A=<redacted> timeout -s <redacted> <redacted> \
             rm -rf /",
        ),
        ("/usr/bin/sudo rm -rf /", "sudo rm -rf /"),
    ]);
}

#[test]
fn a_literal_shell_body_is_rendered_as_commands_and_any_other_is_hidden() {
    assert_cases(&[
        (
            "bash -c 'rm -rf / && git push -f origin main'",
            "bash -c 'rm -rf /; git push -f origin main'",
        ),
        ("bash -c \"$CMD\"", "bash -c <redacted>"),
        ("eval \"$CMD\"", "eval <redacted>"),
        ("eval rm -rf /", "eval 'rm -rf /'"),
        ("bash -c $'rm -rf /\\nls'", "bash -c 'rm -rf /; ls'"),
        ("bash -c 'echo \"open'", "bash -c '<unparsed>'"),
        (
            "su -c 'dropdb prod' root",
            "su -c 'dropdb <redacted>' <redacted>",
        ),
        // env splits `-S`'s string into words and runs them without a shell.
        ("env -Srm\\ -rf\\ /", "env -<redacted>"),
        (
            "su --command='rm -rf /'",
            "su --command=<redacted> 'rm -rf /'",
        ),
    ]);
}

#[test]
fn nested_bodies_are_rendered_down_to_the_nesting_limit() {
    let shallow = format!("{} dropdb production", "eval ".repeat(3));
    let deep = format!("{} dropdb production", "eval ".repeat(40));

    let shallow = redact(&shallow);
    let deep = redact(&deep);

    assert_eq!(shallow, "eval 'eval 'eval 'dropdb <redacted>'''");
    assert!(
        deep.contains(REDACTED) && !deep.contains("dropdb"),
        "{deep}"
    );
}

#[test]
fn find_actions_are_rendered_as_commands() {
    assert_cases(&[
        (
            "find / -name '*.log' -exec rm -rf {} \\;",
            "find / -name <redacted> -exec rm -rf {} ;",
        ),
        (
            "find . -delete -execdir sh -c 'rm -f x' \\;",
            "find . -delete -execdir sh -c 'rm -f <redacted>' ;",
        ),
    ]);
}

#[test]
fn d1_keeps_the_signal_of_destructive_commands() {
    assert_cases(&[
        (
            "git push --force origin main",
            "git push --force origin main",
        ),
        ("rm -rf /", "rm -rf /"),
        ("kubectl delete ns prod", "kubectl delete ns <redacted>"),
        (
            "terraform destroy -auto-approve",
            "terraform destroy -auto-approve",
        ),
        (
            "gh pr merge 5 --repo github.com/o/r --match-head-commit abc123",
            "gh pr merge <redacted> --repo <redacted> --match-head-commit <redacted>",
        ),
        (
            "gh repo delete o/r --yes",
            "gh repo delete <redacted> --yes",
        ),
        ("docker rmi -f img", "docker rmi -f <redacted>"),
        ("git filter-branch -- --all", "git filter-branch -- --all"),
    ]);
}

// Review round 1 of PR 120.

#[test]
fn a_value_or_operand_naming_a_wrapper_is_hidden_like_any_other() {
    assert_cases(&[
        (
            "find /home/alice/acme/env -delete",
            "find <redacted> -delete",
        ),
        ("bash /opt/x/script", "bash <redacted>"),
        ("source dir/env", "source <redacted>"),
        (". dir/sudo", ". <redacted>"),
        ("sudo -D /x/nice rm", "sudo -D <redacted> rm"),
        ("script -c x /logs/s/env", "script -c 'x' <redacted>"),
        ("flock /tmp/env rm", "flock <redacted> rm"),
        ("function nice", "function <redacted>"),
    ]);
}

#[test]
fn a_program_or_wrapper_is_shown_by_its_name_without_its_path() {
    assert_cases(&[
        ("./deploy.sh x", "deploy.sh <redacted>"),
        ("/opt/alice/bin/env -i rm", "env -i rm"),
        ("timeout 5 /x/acme/nice rm", "timeout <redacted> nice rm"),
        ("/x/acme/sudo.exe rm -rf /", "sudo rm -rf /"),
    ]);
}

#[test]
fn find_keeps_its_flag_tables_under_a_wrapper_or_an_assignment() {
    assert_cases(&[
        (
            "sudo find . -name x -exec rm {} \\; -delete",
            "sudo find . -name <redacted> -exec rm {} ; -delete",
        ),
        (
            "X=1 find . -exec rm {} + -print",
            "X=<redacted> find . -exec rm {} + -print",
        ),
    ]);
}

#[test]
fn after_a_literal_double_dash_every_word_is_an_operand() {
    assert_cases(&[
        (
            "grep -- --secret-token-name file",
            "grep -- <redacted> <redacted>",
        ),
        ("rm -- -rf /", "rm -- <redacted> /"),
        ("bash -- --x", "bash -- <redacted>"),
        // env knows no `--x`, so the word after it may be its value.
        (
            "sudo -- env --x=1 rm",
            "sudo -- env --x=<redacted> <redacted>",
        ),
    ]);
}

// Review round 2 of PR 120.

#[test]
fn the_inside_of_an_arithmetic_expression_is_never_a_program_or_argument() {
    assert_cases(&[
        ("sleep $(( 3600 * 24 ))", "<redacted>; sleep <redacted>"),
        ("x=$(( zqVAL ))", "<redacted>; x=<redacted>"),
        ("(( zqX = 1 ))", REDACTED),
        (
            "if (( n > 3 )); then rm -rf /; fi",
            "if; <redacted>; then rm -rf /; fi",
        ),
        (
            "for (( i = 0; i < 3; i++ )); do echo $i; done",
            "for; <redacted>; <redacted>; <redacted>; do echo <redacted>; done",
        ),
        ("x=$[ zqVAL ]", "x=<redacted> <redacted> <redacted>"),
        (
            "sleep $[ 3600 * 24 ] && rm -rf /",
            "sleep <redacted> <redacted> <redacted> <redacted> <redacted>; rm -rf /",
        ),
        (
            "x=$[ (zqX) + 1 ]",
            "x=<redacted>; <redacted>; <redacted> <redacted> <redacted>",
        ),
    ]);
}

#[test]
fn a_substitution_inside_arithmetic_is_still_rendered_as_a_command() {
    assert_cases(&[
        (
            "echo $(( $(rm -rf /) + 1 ))",
            "rm -rf /; <redacted>; echo <redacted>",
        ),
        (
            "x=$[ `dropdb prod` ]",
            "dropdb <redacted>; x=<redacted> <redacted> <redacted>",
        ),
    ]);
}

#[test]
fn a_short_option_cluster_of_a_wrapper_hides_the_rest_of_its_command() {
    // Review round 5: the denylist reads a cluster as one flag, the wrapper may read a value
    // in it (`-iu` is `-i -u`), so the word after it is no certain program.
    assert_cases(&[
        (
            "sudo -iu postgres psql",
            "sudo -<redacted> <redacted> <redacted>",
        ),
        ("sudo -uroot psql", "sudo -<redacted> psql"),
        ("env -iu HOME rm", "env -<redacted> <redacted> <redacted>"),
        ("xargs -0n 1 rm", "xargs -<redacted> <redacted> <redacted>"),
        ("xargs -n1 rm", "xargs -<redacted> rm"),
    ]);
}

#[test]
fn a_wrapper_value_option_the_denylist_does_not_know_hides_the_rest_of_its_command() {
    assert_cases(&[
        (
            "sudo -r role -t type -T 5 -u x psql",
            "sudo -r <redacted> -t <redacted> -T <redacted> -u <redacted> psql",
        ),
        (
            "sudo -a bsd -c staff -R /jail -r role -t type -T 5 -h host psql",
            "sudo -a <redacted> -c <redacted> -R <redacted> -r <redacted> -t <redacted> \
             -T <redacted> -h <redacted> <redacted>",
        ),
        ("doas -a style psql", "doas -a <redacted> <redacted>"),
        ("env -P /opt/bin psql", "env -P <redacted> <redacted>"),
        (
            "xargs -J % -R 2 -S 512 psql",
            "xargs -J <redacted> -R <redacted> -S <redacted> <redacted>",
        ),
    ]);
}

#[test]
fn chrt_or_taskset_with_a_pid_option_hides_the_rest_of_its_command() {
    assert_cases(&[
        ("chrt -p 10 4242", "chrt -p <redacted> <redacted>"),
        ("chrt -f -p 10 4242", "chrt -f -p <redacted> <redacted>"),
        ("taskset -p 0x3 4242", "taskset -p <redacted> <redacted>"),
        ("taskset --pid 4242", "taskset --pid <redacted>"),
        // Review round 10: chrt's words are never read with certainty, pid or none.
        ("chrt -f 10 psql", "chrt -f <redacted> <redacted>"),
        (
            "chrt -p 10 psql x 0",
            "chrt -p <redacted> <redacted> <redacted> <redacted>",
        ),
        (
            "chrt -p 10 psql x -1",
            "chrt -p <redacted> <redacted> <redacted> -1",
        ),
        (
            "taskset -p 0x1 psql x 0",
            "taskset -p <redacted> <redacted> <redacted> <redacted>",
        ),
    ]);
}

/// A deterministic generator of argument tokens.
struct Tokens(u64);

impl Tokens {
    const ALPHABET: &'static [u8] =
        b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_.:@%+,/";

    /// A token no allowlist names: `zq` and 8 characters of [`Self::ALPHABET`].
    fn next(&mut self) -> String {
        let mut token = String::from("zq");
        for _ in 0..8 {
            token.push(self.pick(Self::ALPHABET));
        }
        token
    }

    /// An identifier, `[a-z][a-z0-9_]{2,8}`.
    fn identifier(&mut self) -> String {
        let mut token = String::from(self.pick(b"abcdefghijklmnopqrstuvwxyz"));
        for _ in 0..self.count(2, 8) {
            token.push(self.pick(b"abcdefghijklmnopqrstuvwxyz0123456789_"));
        }
        token
    }

    /// A number, `[0-9]{2,6}`.
    fn digits(&mut self) -> String {
        (0..self.count(2, 6))
            .map(|_| self.pick(b"0123456789"))
            .collect()
    }

    fn count(&mut self, min: usize, max: usize) -> usize {
        min + self.index(max - min + 1)
    }

    fn pick(&mut self, alphabet: &[u8]) -> char {
        char::from(alphabet[self.index(alphabet.len())])
    }

    fn index(&mut self, len: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        usize::try_from(self.0 >> 33).expect("31 bits fit") % len
    }
}

/// Commands with `{}` where a random token goes, in every position no allowlist names.
const TEMPLATES: &[&str] = &[
    "echo {}",
    "rm -rf {}",
    "git push origin {}",
    "curl -H 'X: {}' u",
    "mysql -p{} db",
    "tool --opt={}",
    "tool --opt {}",
    "K={} cmd",
    "cmd > {}",
    "cat < {}",
    "cat <<< {}",
    "x=$(echo {})",
    "bash -c 'echo {}'",
    "eval echo {}",
    "sudo -u {} dropdb",
    "timeout {} dropdb",
    "env -u {} dropdb",
    "find . -name {} -exec rm {} \\;",
    "cat <<EOF\n{}\nEOF",
    "git -C {} push",
    "echo \"{}\"",
    "printf '%s' '{}'",
    "kubectl delete ns {}",
    "docker rm -f {}",
    "ls -{}",
    "rm -r{}",
    "dd of={}",
    "nohup env x={} dropdb {}",
    "bash -c \"tool --k={} | sh\"",
    "echo `echo {}`",
    "find . -name x{}=y",
    "trap 'echo {}' EXIT",
    // Wrapper names in value and operand positions (PR 120 review round 1).
    "find /home/{}/env -delete",
    "find {}/nice -name x",
    "bash /opt/{}/script",
    "source {}/env",
    ". {}/sudo",
    "sudo -D /x/{}/nice rm",
    "sudo -u {}/env rm",
    "script -c x /logs/{}/env",
    "timeout -s {}/nice 5 rm",
    "env -C {}/bash rm",
    "flock /tmp/{}/env rm",
    "xargs -I {}/sh rm",
    "exec -a {}/bash rm",
    "nice -n {}/env rm",
    "find . -exec rm x \\; /a/{}/env",
    "/opt/{}/bin/rm -rf /",
    "sudo /opt/{}/env rm",
    "grep -- --{} file",
    "bash -- --{}",
    // Arithmetic, option clusters and wrapper values (PR 120 review round 2).
    "sleep $(( {} * 24 ))",
    "x=$(( {} ))",
    "(( {} = 1 ))",
    "if (( {} > 3 )); then rm -rf /; fi",
    "for (( i = {}; i < 3; i++ )); do echo; done",
    "x=$[ {} ]",
    "sleep $[ {} * 24 ]",
    "echo $[ ({}) + 1 ]",
    "echo $(( {} + $(echo x) ))",
    "sudo -iu {} psql",
    "sudo -u{} psql",
    "env -iu {} psql",
    "xargs -0n {} psql",
    "sudo -a {} psql",
    "sudo -c {} psql",
    "sudo -R {} psql",
    "sudo -r {} psql",
    "sudo -t {} psql",
    "sudo -T {} psql",
    "sudo -h {} psql",
    "doas -a {} psql",
    "env -P {} psql",
    "xargs -J {} psql",
    "xargs -R {} -I x psql",
    "xargs -S {} psql",
    "chrt -p 10 {}",
    "chrt -p {}",
    "taskset -p 0x3 {}",
    "taskset -p {}",
    // Where the denylist's walk places the program and the wrapper may not (PR 120 review
    // round 5): the token stands where the walk would show a program.
    "xargs -eE {} x",
    "watch -dn {} x",
    "script -tqc {} x",
    "sudo -iu x {}",
    "echo 0 | xargs chrt -p 10 {} 4242",
    "taskset -cp 0x1 {} 4242",
    "nohup K=v {}",
    "doas K=v {}",
    "sudo -e {}",
    "sudo -h x {}",
    "time -o x {}",
    "env -iC x {}",
    "env - {}",
    "command -v {}",
    "ionice -p 1 {}",
    "timeout --kill 5 {}",
    "bash -lc 'echo {}'",
    "bash -lsc {}",
    "su -ls x -c {}",
    "flock -nw 5 x {}",
    // A value or operand without its slot's shape (PR 120 review round 6): chrt's priority
    // is optional under these policies and taken only when all digits.
    "chrt -o x {}",
    "chrt -b x {}",
    "chrt -i x {}",
    "chrt -d x {}",
    "chrt -f x {}",
    "bash -oerrexit -c {}",
    "bash -cl {}",
    "watch -n x {}",
    "watch -n -d {}",
    "nice -n x {}",
    "timeout x {}",
    "taskset x {}",
    "taskset -c ff {}",
    "ionice -c x {}",
    "xargs -n x {}",
    "find . -name -exec {} \\;",
    "find . -exec echo + -exec {} x \\;",
    // Wrappers that split their words and run them without a shell, and slots filled by a
    // word that may expand (PR 120 review round 7).
    "env -S 'echo a; {}'",
    "env --split-string='echo a; {}'",
    "env -S 'echo `{}`'",
    "watch -x echo 'a; {}'",
    "watch --exec echo 'a; {}'",
    "sudo -u $U {}",
    "doas -u u* {}",
    "flock /tmp/*.lock {}",
    "su $U -c {}",
    "find . -name *.rs -exec {} x \\;",
    // A wrapper's assignment that bash may field-split, and a body option followed by another
    // option (PR 120 review round 8).
    "env K=$V {}",
    "sudo K=$V {}",
    "env K=$V x {}",
    "su -c '{} x' -c ls",
    "su -c'{} x' -cls",
    "su --command '{} x' --command ls",
    "su --command='echo a' --command='{} b'",
    "script -c '{} x' -c ls /dev/null",
    "script -c'{} x' -cls /dev/null",
    "script --command '{} x' --command ls /dev/null",
    "script --command='{} x' --command=ls /dev/null",
    // Words after (or before) a body that keep its wrapper from running it (PR 120 review
    // round 9).
    "trap '{} x'",
    "trap '{} x' -l",
    "trap '{} x' $S",
    "trap 2 '{} x'",
    "trap - '{} x'",
    "flock /l -c '{} x' y",
    "flock /l --command '{} x' y",
    "flock /l --command='{} x'",
    "script -c '{} x' a b",
    "script a -c '{} x' b",
    "script --command='{} x' a b",
    "script -O log -c '{} x' typescript",
    "dash -B -c '{} x'",
    "sh -h -c '{} x'",
    "zsh -b -c '{} x'",
    "mksh -T -c '{} x'",
    "bash -n -c '{} x'",
    "bash -l --norc -c '{} x'",
    "bash -o x -c '{} x'",
    "bash -O x -c '{} x'",
    "su -s /bin/false -c '{} x'",
    "alias 'a b={} x'",
    // A body or program the wrapper may never run, or may run with the replace string put in
    // (PR 120 review round 10, and its self-hunt).
    "trap '{} x' EXIT",
    "trap '{}' NOSUCH",
    "sh -c \"trap '{} x' INT\"",
    "alias a='{} x'",
    "xargs -I {} bash -c '{} x'",
    "xargs -I {} sh -c '{}'",
    "xargs -I{} sh -c '{} x'",
    "xargs -i sh -c '{} x'",
    "bash --restricted -c '/x/{} y'",
    "bash -r -c '{} y'",
    "bash -k -c 'sudo -u X=1 {}'",
    "bash -o keyword -c '{} x'",
    "builtin {} x",
    "busybox {} x",
    "function f {} x",
    "coproc {} ( dropdb x )",
    "doas -s {} x",
    "timeout -s NOSUCH 5 {}",
    "xargs -n 0 {}",
    "xargs -d ab {}",
    "xargs -p {}",
    "env -u a=b {}",
    "env -C '' {}",
    "env -0 {}",
    "sudo -C 2 {}",
    "flock -E 300 /l {}",
    "chrt -o 5 {}",
    "taskset 0 {}",
    "stdbuf -i L {}",
    "setsid -c {}",
    "find . -quit -exec {} x \\;",
    "find . -print -o -exec {} x \\;",
    "find . -ok {} x \\;",
    "find . -regex '[' -exec {} x \\;",
    "find '' -exec {} x \\;",
];

#[test]
fn an_argument_no_allowlist_names_never_reaches_the_output() {
    // TST-06 by plain loops: every template, many random tokens.
    let mut tokens = Tokens(0x5eed);
    for template in TEMPLATES {
        for _ in 0..40 {
            let token = tokens.next();
            let command = template.replace("{}", &token);

            let redacted = redact(&command);

            assert_ne!(redacted, UNPARSED, "{command:?} reads");
            assert!(
                !redacted.contains(&token),
                "{command:?} became {redacted:?}"
            );
        }
    }
}

/// Words the lexer reads as reserved wherever they stand, or that change how it reads the
/// rest (`$(echo case)` opens a `case`): a template filled with one is another command.
const RESERVED: &[&str] = &[
    "case", "esac", "for", "select", "while", "until", "if", "then", "else", "elif", "fi", "do",
    "done", "function", "time", "coproc",
];

/// The whole words of `text`: its runs of ASCII letters, digits and `_`.
fn whole_words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|word| !word.is_empty())
}

#[test]
fn an_identifier_or_number_no_allowlist_names_never_reaches_the_output() {
    // TST-06 by plain loops. A token an allow table names may be shown by design, a reserved
    // word changes the command, and one the template's own rendering holds as a word
    // (`echo`, `redacted`) cannot be told apart from it: those are skipped.
    let mut tokens = Tokens(0x5eed_0002);
    for template in TEMPLATES {
        let reference = redact(&template.replace("{}", "zqreference"));
        for _ in 0..30 {
            for token in [tokens.identifier(), tokens.digits()] {
                let skipped = allow::names(&token)
                    || RESERVED.contains(&token.as_str())
                    || whole_words(&reference).any(|word| word == token);
                if skipped {
                    continue;
                }
                let command = template.replace("{}", &token);

                let redacted = redact(&command);

                assert_ne!(redacted, UNPARSED, "{command:?} reads");
                assert!(
                    whole_words(&redacted).all(|word| word != token),
                    "{command:?} became {redacted:?}"
                );
            }
        }
    }
}
