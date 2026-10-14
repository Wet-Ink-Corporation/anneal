//! Table-driven tests: each denylist entry with its allowed near misses, the bypass forms, and
//! commands that cannot be read. The review's bypass repros are in [`bypasses`], forms that
//! reach a command through indirection in [`indirection`], and every wrapping form crossed with
//! every denied command in [`forms`].

mod bypasses;
mod forms;
mod indirection;
mod merge;

use std::ffi::OsString;

use super::{Context, Verdict, check};
use crate::hook::guard::Rule;

const FORCE_PUSH: Option<Rule> = Some(Rule::ForcePush);
const RM_RF: Option<Rule> = Some(Rule::RecursiveForceDelete);
const ADD_FORCE: Option<Rule> = Some(Rule::ForceAdd);
const CONFIG: Option<Rule> = Some(Rule::ConfigWrite);
const UNREADABLE: Option<Rule> = Some(Rule::Unparseable);
const ALLOW: Option<Rule> = None;

/// `$TMPDIR` is `/var/tmp-claude` and the project is `/work/proj`.
fn context() -> Context {
    Context::new(|var| match var {
        "TMPDIR" => Some(OsString::from("/var/tmp-claude")),
        "CLAUDE_PROJECT_DIR" => Some(OsString::from("/work/proj")),
        _ => None,
    })
}

fn rule_for(command: &str) -> Option<Rule> {
    match check(command, &context()) {
        Verdict::Deny(denial) => Some(denial.rule),
        Verdict::Undecided => None,
    }
}

fn assert_table(rows: &[(&str, Option<Rule>)]) {
    let wrong: Vec<String> = rows
        .iter()
        .filter_map(|(command, expected)| {
            let actual = rule_for(command);
            (actual != *expected).then(|| format!("{command:?}: {actual:?}, expected {expected:?}"))
        })
        .collect();
    assert!(wrong.is_empty(), "misjudged:\n{}", wrong.join("\n"));
}

#[test]
fn force_pushes_to_protected_branches_are_denied() {
    assert_table(&[
        ("git push --force origin main", FORCE_PUSH),
        ("git push -f origin master", FORCE_PUSH),
        ("git push -fu origin main", FORCE_PUSH),
        ("git push -uf origin release/1.2", FORCE_PUSH),
        ("git push --force-with-lease origin main", FORCE_PUSH),
        (
            "git push --force-with-lease=main:abc123 origin main",
            FORCE_PUSH,
        ),
        ("git push origin +main", FORCE_PUSH),
        ("git push origin +HEAD:main", FORCE_PUSH),
        ("git push -f origin HEAD:refs/heads/main", FORCE_PUSH),
        ("git push -f origin feature:main", FORCE_PUSH),
        ("git push -f origin 'refs/heads/*'", FORCE_PUSH),
        ("git -C repo push -f origin main", FORCE_PUSH),
        (
            "git -c push.default=current push -f origin main",
            FORCE_PUSH,
        ),
        ("git push origin main --force", FORCE_PUSH),
    ]);
}

#[test]
fn force_pushes_whose_destination_is_not_explicit_are_denied() {
    assert_table(&[
        ("git push --force", FORCE_PUSH),
        ("git push -f origin", FORCE_PUSH),
        ("git push -f origin HEAD", FORCE_PUSH),
        ("git push -f origin @", FORCE_PUSH),
        ("git push -f origin \"$BRANCH\"", FORCE_PUSH),
        ("git push --force --all origin", FORCE_PUSH),
        ("git push --mirror origin", FORCE_PUSH),
    ]);
}

#[test]
fn ordinary_pushes_and_force_pushes_to_other_branches_are_allowed() {
    assert_table(&[
        ("git push origin main", ALLOW),
        ("git push", ALLOW),
        ("git push -u origin feature", ALLOW),
        ("git push --force origin feature", ALLOW),
        ("git push -f origin HEAD:feature", ALLOW),
        ("git push -f origin HEAD:refs/heads/wip/14", ALLOW),
        ("git push origin +feature", ALLOW),
        ("git push origin +feature main", ALLOW),
        ("git push --force-with-lease origin wip/14", ALLOW),
        ("git push -f origin release-notes", ALLOW),
        ("git push -o ci.skip origin main", ALLOW),
        ("git fetch -f origin main:main", ALLOW),
        ("git pull --force origin main", ALLOW),
        ("echo git push -f origin main", ALLOW),
    ]);
}

#[test]
fn recursive_force_deletes_outside_temp_and_scratch_are_denied() {
    assert_table(&[
        ("rm -rf /", RM_RF),
        ("rm -rf .", RM_RF),
        ("rm -rf ..", RM_RF),
        ("rm -rf *", RM_RF),
        ("rm -fr src", RM_RF),
        ("rm -r -f build", RM_RF),
        ("rm --recursive --force target", RM_RF),
        ("rm -Rf /home/user", RM_RF),
        ("rm -rfv /etc", RM_RF),
        ("rm src -rf", RM_RF),
        ("rm -rf -- /", RM_RF),
        ("rm -rf ~", RM_RF),
        ("rm -rf ~/projects", RM_RF),
        ("rm -rf $HOME/x", RM_RF),
        ("rm -rf \"$TMPDIR/x\"", RM_RF),
        ("rm -rf /tmp", RM_RF),
        ("rm -rf /tmp/", RM_RF),
        ("rm -rf /tmp/../etc", RM_RF),
        ("rm -rf /tmp/*", RM_RF),
        ("rm -rf /tmp/{a,b}", RM_RF),
        ("rm -rf /tmp/a /etc", RM_RF),
        ("rm -rf tmp/x", RM_RF),
        ("rm -rf /work/proj/.scratch", RM_RF),
        ("rm -rf /work/proj/src", RM_RF),
    ]);
}

#[test]
fn deletes_under_temp_or_scratch_and_non_recursive_or_unforced_deletes_are_allowed() {
    assert_table(&[
        ("rm -rf /tmp/build", ALLOW),
        ("rm -rf /tmp/a /tmp/b", ALLOW),
        ("rm -rf '/tmp/with space'", ALLOW),
        ("rm -rf /var/tmp-claude/x", ALLOW),
        ("rm -rf /var/folders/ab/T/x", ALLOW),
        ("rm -rf /private/tmp/x", ALLOW),
        ("rm -rf /work/proj/.scratch/run-1", ALLOW),
        ("rm -rf /tmp/./x", ALLOW),
        ("rm -r src", ALLOW),
        ("rm -f file.txt", ALLOW),
        ("rm file", ALLOW),
        ("rm -rf", ALLOW),
        ("rmdir build", ALLOW),
        ("echo rm -rf /", ALLOW),
        ("grep -r \"rm -rf\" .", ALLOW),
    ]);
}

#[test]
fn git_add_with_force_is_denied_and_plain_adds_are_allowed() {
    assert_table(&[
        ("git add -f target/x", ADD_FORCE),
        ("git add --force .", ADD_FORCE),
        ("git add -fA", ADD_FORCE),
        ("git add -Af", ADD_FORCE),
        ("git -C x add -f y", ADD_FORCE),
        ("git add .", ALLOW),
        ("git add -A", ALLOW),
        ("git add -p", ALLOW),
        ("git add -- -f", ALLOW),
        ("git add file-f", ALLOW),
        ("git rm -f x", ALLOW),
        ("git commit -a -m 'add -f'", ALLOW),
    ]);
}

#[test]
fn bash_writes_to_lint_or_hook_config_are_denied() {
    assert_table(&[
        ("echo x > .claude/settings.json", CONFIG),
        ("echo x >> .claude/settings.local.json", CONFIG),
        ("echo x > ./.claude/settings.json", CONFIG),
        ("cat a > clippy.toml", CONFIG),
        ("printf x > crates/a/.clippy.toml", CONFIG),
        ("echo > rustfmt.toml", CONFIG),
        ("echo > .rustfmt.toml", CONFIG),
        ("echo x 2> plugin/hooks/hooks.json", CONFIG),
        ("cmd &> .git/hooks/pre-commit", CONFIG),
        ("echo x > .cl*/settings.json", CONFIG),
        ("tee deny.toml < x", CONFIG),
        ("echo x | tee -a .cargo/config.toml", CONFIG),
        ("sed -i 's/a/b/' .github/workflows/ci.yml", CONFIG),
        ("sed -i.bak -e 's/a/b/' lefthook.yml", CONFIG),
        ("sed --in-place 's/a/b/' .eslintrc.json", CONFIG),
        ("cp x hooks/pre-commit", CONFIG),
        ("mv a .claude/hooks/h.sh", CONFIG),
        ("ln -sf x .pre-commit-config.yaml", CONFIG),
        ("cp settings.json .claude/", CONFIG),
        ("cp -t .claude settings.local.json", CONFIG),
        ("install -m 644 x .cargo/config", CONFIG),
        ("mv .claude/settings.json backup.json", CONFIG),
        ("rm hooks/x.sh", CONFIG),
        ("rm -r .github", CONFIG),
        ("truncate -s 0 .eslintrc.json", CONFIG),
        ("chmod -x .claude/hooks/check.sh", CONFIG),
        ("echo x > .Claude/Settings.json", CONFIG),
        ("cp x .GitHub/Workflows/x.yml", CONFIG),
    ]);
}

#[test]
fn bash_writes_to_the_anneal_config_are_denied_and_the_rest_of_anneal_is_not() {
    assert_table(&[
        ("echo '{}' > .anneal/config.json", CONFIG),
        ("echo x >> ./.anneal/config.json", CONFIG),
        ("tee .anneal/config.json < x", CONFIG),
        ("sed -i 's/a/b/' .anneal/config.json", CONFIG),
        ("cp x.json .anneal/config.json", CONFIG),
        ("mv .anneal/config.json old.json", CONFIG),
        ("rm .anneal/config.json", CONFIG),
        ("rm -r .anneal", CONFIG),
        ("echo x > .Anneal/Config.json", CONFIG),
        ("echo x > .anneal/briefs/36.md", ALLOW),
        ("cat .anneal/config.json", ALLOW),
        ("echo x > config.json", ALLOW),
    ]);
}

#[test]
fn a_hard_link_to_protected_config_is_a_write_of_it() {
    assert_table(&[
        ("ln .anneal/config.json x", CONFIG),
        ("ln -f .anneal/config.json x", CONFIG),
        ("ln -v -- .anneal/config.json x", CONFIG),
        ("ln .anneal/config.json", CONFIG),
        ("ln -t /tmp .anneal/config.json", CONFIG),
        ("ln .claude/settings.json /tmp/s.json", CONFIG),
        ("ln --suffix=.bak .claude/settings.local.json x", CONFIG),
        ("ln --s=.bak .claude/settings.local.json x", CONFIG),
        ("ln -Ss .claude/settings.json x", CONFIG),
        ("link .anneal/config.json x", CONFIG),
        ("cp -l .anneal/config.json x", CONFIG),
        ("cp -al .claude /tmp/c", CONFIG),
        ("cp --link .claude/settings.json x", CONFIG),
        ("sudo ln .anneal/config.json x", CONFIG),
        ("ln -s .anneal/config.json x", ALLOW),
        ("ln -sf .claude/settings.json x", ALLOW),
        ("ln --sym .claude/settings.json x", ALLOW),
        ("ln notes.md x", ALLOW),
        ("cp .anneal/config.json x", ALLOW),
        ("cp -s .anneal/config.json x", ALLOW),
    ]);
}

#[test]
fn reads_and_writes_elsewhere_are_allowed() {
    assert_table(&[
        ("cat .claude/settings.json", ALLOW),
        ("cp .claude/settings.json /tmp/s.json", ALLOW),
        ("cp clippy.toml /tmp/x", ALLOW),
        ("echo x > .claude/notes.md", ALLOW),
        ("echo x > myhooks/a", ALLOW),
        ("echo x > docs/hooks.md", ALLOW),
        ("echo x > clippy.toml.bak", ALLOW),
        ("sed 's/hooks/x/' file > out", ALLOW),
        ("sed -i 's/hooks/x/' notes.txt", ALLOW),
        ("sed -n p clippy.toml", ALLOW),
        ("tee out.txt", ALLOW),
        ("git diff > .claude/plan.md", ALLOW),
        ("cp -r .claude/skills /tmp/skills", ALLOW),
        ("ln -s .claude/settings.json", ALLOW),
        ("ls hooks/", ALLOW),
    ]);
}

#[test]
fn the_five_bypass_forms_are_denied() {
    assert_table(&[
        // Quoting.
        ("\"git\" \"push\" \"--force\" origin main", FORCE_PUSH),
        ("g'i't push -f origin main", FORCE_PUSH),
        ("r\\m -rf /", RM_RF),
        ("rm \"-rf\" /", RM_RF),
        ("rm -r''f /", RM_RF),
        // bash -c and its relatives.
        ("bash -c 'git push -f origin main'", FORCE_PUSH),
        ("sh -c \"rm -rf /\"", RM_RF),
        ("zsh -lc 'git add -f x'", ADD_FORCE),
        ("bash -o pipefail -c 'rm -rf /'", RM_RF),
        ("bash -c \"bash -c 'rm -rf /'\"", RM_RF),
        // Environment prefixes.
        ("FOO=1 git push -f origin main", FORCE_PUSH),
        ("env GIT_X=1 rm -rf /", RM_RF),
        ("env -i PATH=/bin rm -rf /", RM_RF),
        ("env -S 'rm -rf /'", RM_RF),
        // Chains.
        ("cd x && git push -f origin main", FORCE_PUSH),
        ("true || rm -rf /", RM_RF),
        ("ls; rm -rf /", RM_RF),
        ("ls | rm -rf /", RM_RF),
        ("sleep 1 & rm -rf /", RM_RF),
        // Subshells and groups.
        ("(rm -rf /)", RM_RF),
        ("(cd x && (git push -f origin main))", FORCE_PUSH),
        ("{ rm -rf /; }", RM_RF),
    ]);
}

#[test]
fn substitutions_eval_paths_newlines_and_wrappers_do_not_hide_a_command() {
    assert_table(&[
        ("echo $(rm -rf /)", RM_RF),
        ("x=\"$(git push -f origin main)\"", FORCE_PUSH),
        ("echo `rm -rf /`", RM_RF),
        ("cat <(rm -rf /)", RM_RF),
        ("eval 'rm -rf /'", RM_RF),
        ("eval rm -rf /", RM_RF),
        ("eval \"git push\" -f origin main", FORCE_PUSH),
        ("/bin/rm -rf /", RM_RF),
        ("/usr/bin/git push -f origin main", FORCE_PUSH),
        ("ls\nrm -rf /", RM_RF),
        ("ls \\\n && rm -rf /", RM_RF),
        ("sudo rm -rf /", RM_RF),
        ("sudo -u root rm -rf /", RM_RF),
        ("nohup rm -rf / &", RM_RF),
        ("time rm -rf /", RM_RF),
        ("command rm -rf /", RM_RF),
        ("exec rm -rf /", RM_RF),
        ("timeout 5 rm -rf /", RM_RF),
        ("nice -n 5 git push -f origin main", FORCE_PUSH),
        ("xargs rm -rf < list", RM_RF),
        ("find . -name x | xargs rm -rf", RM_RF),
        ("find . -name x -exec rm -rf {} +", RM_RF),
        ("$(echo rm) -rf /", RM_RF),
        ("\"$GIT\" push -f origin main", FORCE_PUSH),
        ("$'\\x72\\x6d' -rf /", RM_RF),
        ("if true; then rm -rf /; fi", RM_RF),
        ("while true; do git push -f origin main; done", FORCE_PUSH),
        ("! rm -rf /", RM_RF),
        ("git push -f origin main # just a comment", FORCE_PUSH),
        ("cat <<EOF\n$(rm -rf /)\nEOF", RM_RF),
    ]);
}

#[test]
fn the_probes_of_every_redaction_review_round_get_main_s_verdict() {
    // PR 120 changed only the redaction; these are the commands its review rounds probed the
    // denylist with, each judged as main (603fa75) judges it. The redaction may hide more
    // than needed, never change what the denylist denies.
    assert_table(&[
        ("sudo -iu x rm -rf /", RM_RF),
        ("sudo -uecho rm -rf /", RM_RF),
        ("xargs -eE rm -rf /", RM_RF),
        ("xargs -R 1 -I echo rm -rf", RM_RF),
        ("watch -dn rm -rf /", RM_RF),
        ("script -tqc x rm -rf /", ALLOW),
        ("env -vu X -C /tmp rm -rf /", RM_RF),
        ("chrt -f 10 rm -rf /", RM_RF),
        ("chrt -p 10 4242", ALLOW),
        ("taskset -p 0x3 4242", ALLOW),
        ("echo 0 | xargs chrt -p 10 rm -rf / 4242", RM_RF),
        ("chrt -p 10 rm -rf / 4242", RM_RF),
        ("chrt -o -p 0 rm -rf / 0", RM_RF),
        ("chrt -o -p 0 rm -rf / -1", RM_RF),
        ("chrt -p 10 rm -rf / 00", RM_RF),
        ("chrt -p 10 rm -rf / +0", RM_RF),
        ("chrt -p 10 rm -rf / -01", RM_RF),
        ("chrt -p 10 rm -rf / ' -1'", RM_RF),
        ("chrt -p 10 rm -rf / -5", RM_RF),
        ("chrt -p 10 rm -rf / $PID", RM_RF),
        ("chrt -p 10 rm -rf / 0x10", RM_RF),
        ("chrt -p 10 rm -rf / ' 4242'", RM_RF),
        ("chrt -p 10 rm -rf / +4242", RM_RF),
        ("taskset -p 0x1 rm -rf / 0", RM_RF),
        ("taskset -p 0x1 rm -rf / -1", RM_RF),
        ("taskset -cp 0 rm -rf / -0", RM_RF),
        // Round 7.
        ("env -S 'echo a; rm -rf /'", RM_RF),
        ("env --split-string='echo a; rm -rf /'", RM_RF),
        ("env -S 'echo `rm -rf /`'", RM_RF),
        ("watch -x echo 'a; rm -rf /'", RM_RF),
        ("watch --exec echo 'a; rm -rf /'", RM_RF),
        ("sudo -u $U rm -rf /", RM_RF),
        ("nice -n $N rm -rf /", RM_RF),
        ("doas -u u* rm -rf /", RM_RF),
        ("flock /tmp/*.lock rm -rf /", RM_RF),
        ("su $U -c 'rm -rf /'", RM_RF),
        ("find . -name *.rs -exec rm -rf / \\;", RM_RF),
        // Round 8. The ALLOW rows are main's gaps (the last `-c` wins), kept as main has them.
        ("env K=$V rm -rf /", RM_RF),
        ("sudo K=$V rm -rf /", RM_RF),
        ("env A=1 K=$V rm -rf /", RM_RF),
        ("su -c 'rm -rf /' -c ls", RM_RF),
        ("su -c ls -c 'rm -rf /'", ALLOW),
        ("su -c'rm -rf /' -cls", ALLOW),
        ("su --command 'rm -rf /' --command ls", RM_RF),
        ("su --command='echo a' --command='rm -rf /'", ALLOW),
        ("script -c 'rm -rf /' -c ls /dev/null", RM_RF),
        ("script -c ls -c 'rm -rf /' /dev/null", ALLOW),
        ("script -c'rm -rf /' -cls /dev/null", RM_RF),
        ("script --command 'rm -rf /' --command ls /dev/null", RM_RF),
        (
            "script --command='echo a' --command='rm -rf /' /dev/null",
            ALLOW,
        ),
        ("bash -c 'rm -rf /' -- -y", RM_RF),
        // Round 9. The ALLOW rows are main's: bash's trap reads a first word of digits or `-`
        // as a signal to reset, and so does the denylist.
        ("trap 'rm -rf /'", RM_RF),
        ("trap 'rm -rf /' -l", RM_RF),
        ("trap 'rm -rf /' $S", RM_RF),
        ("trap 2 'rm -rf /'", ALLOW),
        ("trap - 'rm -rf /'", ALLOW),
        ("flock /l -c 'rm -rf /' y", RM_RF),
        ("flock /l --command 'rm -rf /' y", RM_RF),
        ("flock /l --command='rm -rf /'", RM_RF),
        ("script -c 'rm -rf /' a b", RM_RF),
        ("script a -c 'rm -rf /' b", RM_RF),
        ("script --command='rm -rf /' a b", RM_RF),
        ("script -O log -c 'rm -rf /' typescript", RM_RF),
        ("dash -B -c 'rm -rf /'", RM_RF),
        ("sh -h -c 'rm -rf /'", RM_RF),
        ("zsh -b -c 'rm -rf /'", RM_RF),
        ("mksh -T -c 'rm -rf /'", RM_RF),
        ("bash -n -c 'rm -rf /'", RM_RF),
        ("bash -l --norc -c 'rm -rf /'", RM_RF),
        ("bash -o nosuch -c 'rm -rf /'", RM_RF),
        ("bash -O nosuch -c 'rm -rf /'", RM_RF),
        ("su -s /bin/false -c 'rm -rf /'", RM_RF),
        ("alias 'a b=rm -rf /'", RM_RF),
        // Round 10 and its self-hunt. The ALLOW rows are main's: the walk reads `Z` as the
        // program, and xargs runs the input word put in its place.
        ("trap 'rm -rf /' EXIT", RM_RF),
        ("trap 'rm -rf /' NOSUCH", RM_RF),
        ("sh -c \"trap 'rm -rf /' INT\"", RM_RF),
        ("trap -- 'rm -rf /' INT 15", RM_RF),
        ("alias ll='rm -rf /'", RM_RF),
        ("xargs -I Z bash -c 'Z -rf /'", ALLOW),
        ("xargs -I Z sh -c 'Z'", ALLOW),
        ("xargs -IZ sh -c 'rm -rf /'", RM_RF),
        ("xargs -i sh -c 'rm -rf /'", RM_RF),
        ("bash --restricted -c '/x/rm -rf /'", RM_RF),
        ("bash -r -c 'rm -rf /'", RM_RF),
        ("bash -k -c 'sudo -u X=1 rm -rf /'", RM_RF),
        ("bash -o keyword -c 'rm -rf /'", RM_RF),
        ("builtin rm -rf /", RM_RF),
        ("busybox rm -rf /", RM_RF),
        ("function f rm -rf /", RM_RF),
        ("coproc rm -rf /", RM_RF),
        ("coproc x ( rm -rf / )", RM_RF),
        ("doas -s rm -rf /", RM_RF),
        ("timeout -s NOSUCH 5 rm -rf /", RM_RF),
        ("timeout -s 65 5 rm -rf /", RM_RF),
        ("xargs -n 0 rm -rf /", RM_RF),
        ("xargs -L 0 rm -rf /", RM_RF),
        ("xargs -s 1 rm -rf /", RM_RF),
        ("xargs -d ab rm -rf /", RM_RF),
        ("xargs --process-slot-var=a=b rm -rf /", RM_RF),
        ("xargs -p rm -rf /", RM_RF),
        ("xargs -P 99999999999 rm -rf /", RM_RF),
        ("env -u a=b rm -rf /", RM_RF),
        ("env -C '' rm -rf /", RM_RF),
        ("env -0 rm -rf /", RM_RF),
        ("sudo -C 2 rm -rf /", RM_RF),
        ("flock -E 300 /l rm -rf /", RM_RF),
        ("chrt -o 5 rm -rf /", RM_RF),
        ("taskset 0 rm -rf /", RM_RF),
        ("stdbuf -i L rm -rf /", RM_RF),
        ("stdbuf -o 1Z rm -rf /", RM_RF),
        ("nice -n 99999999999999999999 rm -rf /", RM_RF),
        (
            "script -o 99999999999999999999 -c 'rm -rf /' /dev/null",
            RM_RF,
        ),
        ("setsid -c rm -rf /", RM_RF),
        ("find . -quit -exec rm -rf / \\;", RM_RF),
        ("find . -false -exec rm -rf / \\;", RM_RF),
        ("find . -print -o -exec rm -rf / \\;", RM_RF),
        ("find . -ok rm -rf / \\;", RM_RF),
        ("find . -regex '[' -exec rm -rf / \\;", RM_RF),
        ("find . -perm +644 -exec rm -rf / \\;", RM_RF),
        ("find . -type D -exec rm -rf / \\;", RM_RF),
        ("find . -printf % -exec rm -rf / \\;", RM_RF),
        ("find '' -exec rm -rf / \\;", RM_RF),
        ("find . -maxdepth 99999999999 -exec rm -rf / \\;", RM_RF),
        ("find . -files0-from f -exec rm -rf / \\;", RM_RF),
    ]);
}

#[test]
fn arithmetic_is_read_as_before_and_its_substitutions_checked() {
    assert_table(&[
        ("echo $(( $(rm -rf /) + 1 ))", RM_RF),
        ("echo $[ `rm -rf /` ]", RM_RF),
        ("echo $(( 3600 * 24 ))", ALLOW),
        ("(( x = 1 ))", ALLOW),
        ("for (( i = 0; i < 3; i++ )); do echo $i; done", ALLOW),
        ("x=$[ 1 + 2 ]", ALLOW),
    ]);
}

#[test]
fn quoted_mentions_of_a_denied_command_are_allowed() {
    assert_table(&[
        ("echo 'rm -rf /'", ALLOW),
        ("bash -c 'echo hi'", ALLOW),
        ("bash script.sh", ALLOW),
        (
            "git commit -m \"$(cat <<'EOF'\nDon't force push (git push -f origin main)\nEOF\n)\"",
            ALLOW,
        ),
        ("cat <<'EOF' > /tmp/notes\nrm -rf /\nEOF", ALLOW),
        ("case $1 in start) run;; esac", ALLOW),
    ]);
}

#[test]
fn commands_that_cannot_be_read_are_denied() {
    let deep_eval = (0..10).fold("ls".to_owned(), |inner, _| {
        format!("eval {}", double_quote(&inner))
    });
    let deep_bash = (0..10).fold("ls".to_owned(), |inner, _| {
        format!("bash -c {}", double_quote(&inner))
    });
    assert_table(&[
        ("echo 'unclosed", UNREADABLE),
        ("rm -rf \"/", UNREADABLE),
        ("echo $(ls", UNREADABLE),
        ("echo >", UNREADABLE),
        (&"$(".repeat(40), UNREADABLE),
        (&deep_eval, UNREADABLE),
        (&deep_bash, UNREADABLE),
    ]);
}

/// `text` in double quotes, as a shell reads it back.
fn double_quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', r"\\").replace('"', "\\\""))
}

#[test]
fn nesting_up_to_the_cap_is_still_read() {
    let nested = (0..3).fold("rm -rf /".to_owned(), |inner, _| {
        format!("bash -c {}", double_quote(&inner))
    });

    assert_eq!(rule_for(&nested), RM_RF, "{nested}");
}

#[test]
fn a_denial_names_the_target_and_why() {
    let Verdict::Deny(denial) = check("rm -rf /etc", &context()) else {
        panic!("rm -rf /etc is denied");
    };

    assert_eq!(
        denial.reason,
        "rm -rf /etc: it is outside the temporary and scratch directories; only literal \
         absolute paths under $TMPDIR, /tmp, /var/folders or $CLAUDE_PROJECT_DIR/.scratch may \
         be force-deleted recursively",
        "one line naming the target"
    );
}
