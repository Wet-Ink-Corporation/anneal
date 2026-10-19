//! Every denied command in every wrapping, quoting or substitution form is still denied, and
//! the same forms around a harmless command are not.

use super::{ADD_FORCE, CONFIG, FORCE_PUSH, RM_RF, UNREADABLE, double_quote, rule_for};
use crate::hook::guard::Rule;

/// Denied commands and the rule each one breaks.
const BASES: [(&str, Option<Rule>); 6] = [
    ("rm -rf /", RM_RF),
    ("git push -f origin main", FORCE_PUSH),
    ("git add -f x", ADD_FORCE),
    ("echo x > .claude/settings.json", CONFIG),
    // Abbreviated long options.
    ("rm --recur --forc /", RM_RF),
    ("git push --force-w origin main", FORCE_PUSH),
];

/// `text` in single quotes, as a shell reads it back.
fn single_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// A way to wrap, quote or hide a command: its name and how it wraps one.
type Form = (&'static str, fn(&str) -> String);

/// The ways a command can be wrapped, quoted or hidden.
pub(super) fn forms() -> Vec<Form> {
    let mut forms = shell_forms();
    forms.extend(wrapper_forms());
    forms
}

/// Quoting, substitutions, compound commands, chains and command strings.
fn shell_forms() -> Vec<Form> {
    vec![
        ("bare", |c| c.to_owned()),
        ("bash -c", |c| format!("bash -c {}", single_quote(c))),
        ("sh -c double", |c| format!("sh -c {}", double_quote(c))),
        ("zsh -lc", |c| format!("zsh -lc {}", single_quote(c))),
        ("eval", |c| format!("eval {}", single_quote(c))),
        ("eval bare", |c| format!("eval {c}")),
        ("$( )", |c| format!("echo \"$({c})\"")),
        ("backticks", |c| format!("echo `{c}`")),
        ("${:-$( )}", |c| format!("echo ${{x:-$({c})}}")),
        ("<( )", |c| format!("cat <({c})")),
        ("subshell", |c| format!("({c})")),
        ("group", |c| format!("{{ {c}; }}")),
        ("env prefix", |c| format!("FOO=1 BAR=2 {c}")),
        ("env", |c| format!("env -i A=b {c}")),
        ("env -S", |c| format!("env -S {}", single_quote(c))),
        ("sudo", |c| format!("sudo -u root {c}")),
        ("xargs", |c| format!("echo | xargs {c}")),
        ("find -exec", |c| format!("find . -exec {c} \\;")),
        ("second -exec", |c| {
            format!("find . -exec true \\; -exec {c} \\;")
        }),
        ("heredoc to bash", |c| format!("bash <<'EOF'\n{c}\nEOF")),
        ("here-string", |c| format!("bash <<< {}", single_quote(c))),
        ("pipe to sh", |c| format!("echo {} | sh", single_quote(c))),
        ("function", |c| format!("function f {{ {c}; }}; f")),
        ("f()", |c| format!("f() {{ {c}; }}; f")),
        ("case in $( )", |c| {
            format!("echo \"$(case x in x) {c};; esac)\"")
        }),
        ("case", |c| format!("case x in x) {c};; esac")),
        ("if", |c| format!("if true; then {c}; fi")),
        ("while", |c| format!("while true; do {c}; done")),
        ("!", |c| format!("! {c}")),
        ("&&", |c| format!("true && {c}")),
        ("||", |c| format!("false || {c}")),
        (";", |c| format!("ls; {c}")),
        ("|", |c| format!("ls | {c}")),
        ("&", |c| format!("sleep 1 & {c}")),
        ("newline", |c| format!("ls\n{c}")),
        ("continuation", |c| format!("ls \\\n&& {c}")),
        ("comment after", |c| format!("{c} # done")),
    ]
}

/// Programs that run another command.
fn wrapper_forms() -> Vec<Form> {
    vec![
        ("coproc", |c| format!("coproc {c}")),
        ("time", |c| format!("time -p {c}")),
        ("nohup", |c| format!("nohup {c} &")),
        ("command", |c| format!("command {c}")),
        ("exec -a", |c| format!("exec -a x {c}")),
        ("nice", |c| format!("nice -n 5 {c}")),
        ("timeout", |c| format!("timeout -s KILL 5 {c}")),
        ("setsid", |c| format!("setsid {c}")),
        ("stdbuf", |c| format!("stdbuf -oL {c}")),
        ("ionice", |c| format!("ionice -c 3 {c}")),
        ("busybox", |c| format!("busybox {c}")),
        ("trap", |c| format!("trap {} EXIT", single_quote(c))),
        ("su -c", |c| format!("su -c {} root", single_quote(c))),
        ("script -c", |c| {
            format!("script -q -c {} /dev/null", single_quote(c))
        }),
        ("watch", |c| format!("watch {}", single_quote(c))),
        ("caffeinate", |c| format!("caffeinate {c}")),
        ("runuser", |c| format!("runuser -u root -- {c}")),
    ]
}

/// Whether wrapping `inner` in the form `outer` still runs `inner`. It does not when `eval`
/// re-splits a quoted command (`eval bash -c 'x y'` runs `bash -c x` with `y` as `$0`), when
/// backticks would nest unescaped, or when a pipe ends `find -exec`'s command and leaves the
/// shell a file operand (`find . -exec echo 'x' | sh \;` runs the file `;`).
pub(super) fn still_runs(outer: &str, inner: &str) -> bool {
    match outer {
        "eval bare" => !inner.contains(['\'', '"']),
        "backticks" => !inner.contains('`'),
        "find -exec" | "second -exec" => !inner.contains("| sh"),
        _ => true,
    }
}

#[test]
fn every_denied_command_stays_denied_in_every_pair_of_forms() {
    let forms = forms();
    let mut missed = Vec::new();
    for (base, _) in BASES {
        for (inner_name, inner) in &forms {
            let inner = inner(base);
            for (outer_name, outer) in &forms {
                if !still_runs(outer_name, &inner) {
                    continue;
                }
                let command = outer(&inner);
                if rule_for(&command).is_none() {
                    missed.push(format!("{outer_name} of {inner_name}: {command:?}"));
                }
            }
        }
    }

    assert!(missed.is_empty(), "allowed:\n{}", missed.join("\n"));
}

#[test]
fn every_denied_command_stays_denied_in_every_wrapping_form() {
    let mut missed = Vec::new();
    for (base, rule) in BASES {
        for (name, form) in forms() {
            let command = form(base);
            let actual = rule_for(&command);
            if actual != rule {
                missed.push(format!("{name}: {command:?} -> {actual:?}"));
            }
        }
    }

    assert!(
        missed.is_empty(),
        "allowed or misjudged:\n{}",
        missed.join("\n")
    );
}

#[test]
fn every_wrapping_form_of_a_harmless_command_is_allowed() {
    let denied: Vec<String> = forms()
        .into_iter()
        .filter_map(|(name, form)| {
            let command = form("echo hi");
            rule_for(&command).map(|rule| format!("{name}: {command:?} -> {rule:?}"))
        })
        .collect();

    assert!(denied.is_empty(), "denied:\n{}", denied.join("\n"));
}

#[test]
fn everyday_commands_are_allowed() {
    let denied: Vec<String> = [
        "cargo test -p anneal-cli 2>&1 | tail -20",
        "git log --oneline -5 | cat",
        "for f in src/*.rs; do wc -l \"$f\"; done",
        "find . -name '*.rs' -exec grep -l foo {} +",
        "ls -la && cd crates && cargo build",
        "git commit -m \"$(cat <<'EOF'\nfix: x\nEOF\n)\"",
        "while read -r line; do echo \"$line\"; done < file.txt",
        "export PATH=\"$HOME/.cargo/bin:$PATH\"; cargo --version",
        "diff <(sort a) <(sort b)",
        "cat <<EOF > /tmp/x.txt\nhello $USER\nEOF",
        "jq -r '.x' file | xargs -I{} echo {}",
        "docker run --rm -v \"$PWD\":/w img sh -c 'cd /w && make'",
        "cd \"$(git rev-parse --show-toplevel)\" && cargo fmt && echo done > notes.md",
        "if [ -f x ]; then echo y; fi",
        "case \"$1\" in a) echo a;; *) echo b;; esac",
        "TMP=$(mktemp -d) && cp x \"$TMP\"/",
        "python3 -c 'print(1)'",
        "git -c core.pager=cat log -3",
    ]
    .into_iter()
    .filter_map(|command| rule_for(command).map(|rule| format!("{command:?} -> {rule:?}")))
    .collect();

    assert!(denied.is_empty(), "denied:\n{}", denied.join("\n"));
}

#[test]
fn re_lexing_is_capped_in_total_across_nested_levels() {
    // Each eval level re-lexes its whole argument: 7 levels of a 1 MiB argument stay under the
    // depth cap but over the total.
    let payload = format!("echo {}", "a".repeat(1024 * 1024));
    let nested = (0..7).fold(payload, |inner, _| format!("eval {}", double_quote(&inner)));

    assert_eq!(rule_for(&nested), UNREADABLE, "too much re-lexing denies");
}
