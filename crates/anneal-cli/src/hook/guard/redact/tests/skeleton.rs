//! The skeleton itself: what each kind of word becomes, wrappers read from the denylist's
//! table, nested bodies, and the signal D1 keeps.

use super::super::{REDACTED, UNPARSED, redact};
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
        (
            "nohup A=1 env B=2 run",
            "nohup A=<redacted> env B=<redacted> run",
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
    "builtin",
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
    "chrt -f 10",
    "taskset 0x1",
    "busybox",
    "coproc",
    "flock /tmp/lock",
    "xargs",
    "xargs -n 1",
    "function f",
    "nohup timeout 5 nice -n 1",
];

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
    "env -S",
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
        .chain([
            format!("trap {quoted} EXIT"),
            format!("find . -exec {command} \\;"),
        ]);
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
        ("env -Srm\\ -rf\\ /", "env -<redacted> 'rm -rf /'"),
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
        ("sudo -- env --x=1 rm", "sudo -- env --x=<redacted> rm"),
    ]);
}

/// A deterministic generator of argument tokens no allowlist names: `zq` and 8 characters.
struct Tokens(u64);

impl Tokens {
    const ALPHABET: &'static [u8] =
        b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_.:@%+,/";

    fn next(&mut self) -> String {
        let mut token = String::from("zq");
        for _ in 0..8 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let index = usize::try_from(self.0 >> 33).expect("31 bits fit") % Self::ALPHABET.len();
            token.push(char::from(Self::ALPHABET[index]));
        }
        token
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
