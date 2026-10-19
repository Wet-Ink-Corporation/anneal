//! Extended globs (`shopt -s extglob`, #128): `@( )`, `+( )`, `!( )`, `?( )` and `*( )` are
//! read as globs that may match a file named `-rf`, or a protected path.

use super::{ALLOW, CONFIG, FORCE_PUSH, RM_RF, UNREADABLE, assert_table};

#[test]
fn an_extglob_operand_may_match_a_file_named_like_rm_s_flags() {
    assert_table(&[
        ("rm @(-rf) /home", RM_RF),
        ("rm +(-)rf /home", RM_RF),
        ("rm !(x) /home", RM_RF),
        ("rm ?(-rf) /home", RM_RF),
        ("rm *(-)rf /home", RM_RF),
        ("rm @(a|-rf) /home", RM_RF),
        ("rm x@(-rf) /home", ALLOW),
        ("rm -f @(a|b).o x", ALLOW),
        ("rm -f -- -@(rf) /home/x", ALLOW),
        // A bare group may match protected config, as `rm -f -- *` may.
        ("rm -f -- @(-rf) /home/x", CONFIG),
        ("bash -O extglob -c 'rm @(-rf) /home'", RM_RF),
        ("shopt -s extglob; rm !(x) /home", RM_RF),
    ]);
}

#[test]
fn a_quoted_parenthesis_does_not_close_an_extglob_group() {
    assert_table(&[
        (r#"rm @(a|")"|-rf) /home"#, RM_RF),
        ("rm @(a|')'|-rf) /home", RM_RF),
        ("rm @(a|(b)|-rf) /home", RM_RF),
    ]);
}

/// Bash ends a group at the first `)` that balances it, which may be a `case` pattern's inside
/// the substitution, so the commands after the group cannot be read reliably.
#[test]
fn a_process_substitution_inside_a_group_is_unreadable() {
    assert_table(&[
        ("echo @(<(rm -rf /))", UNREADABLE),
        ("echo x@(a|>(rm -rf /home))", UNREADABLE),
        ("echo @(<(ls))", UNREADABLE),
        (
            "echo @(<(case x in x) ;; esac) ; rm -rf / ; #))",
            UNREADABLE,
        ),
        (
            "echo @(<(case x in x) ;; esac) ; git push -f origin main ; #))",
            UNREADABLE,
        ),
        (
            "echo @(>(case x in x) ;; esac) ; echo x > .claude/settings.json ; #))",
            UNREADABLE,
        ),
        ("echo @(<(case x in x) ;; esac) ; rm -rf /\n))", UNREADABLE),
        (
            "bash -O extglob -c 'echo @(<(case x in x) ;; esac) ; rm -rf / ; #))'",
            UNREADABLE,
        ),
        // A line continuation between `<` and `(` still opens one.
        ("echo @(<\\\n(rm -rf ~))", UNREADABLE),
        ("echo x@(a|>\\\n(rm -rf ~))", UNREADABLE),
        ("echo @(<\\\n\\\n(ls))", UNREADABLE),
        // Outside a group a process substitution is read as before.
        ("diff <(ls a) <(ls b)", ALLOW),
        ("diff <(rm -rf /) x", RM_RF),
    ]);
}

/// Inside a backtick substitution bash ends the substitution at the group's backtick, so a
/// backtick inside a group, at any depth, is not read as opening another.
#[test]
fn a_backtick_inside_a_group_is_unreadable() {
    assert_table(&[
        ("echo `echo @(a`:`rm -rf ~ # )`", UNREADABLE),
        ("echo `echo @(a`:`git push -f origin main # )`", UNREADABLE),
        (
            "echo `echo @(a`:`echo x > .claude/settings.json # )`",
            UNREADABLE,
        ),
        ("echo `echo @($(a`:`rm -rf ~ # ))`", UNREADABLE),
        (r#"echo `echo @("a`:`rm -rf ~ # ")`"#, UNREADABLE),
        ("echo @(`ls`)", UNREADABLE),
        // Outside a group a backtick substitution is read as before.
        ("echo `echo @(a)` `ls`", ALLOW),
        ("echo `rm -rf /`", RM_RF),
    ]);
}

#[test]
fn an_extglob_target_is_a_pattern_for_rm_rf_and_the_config_entries() {
    assert_table(&[
        ("rm -rf /tmp/@(a|b)", RM_RF),
        ("rm -rf @(a b)", RM_RF),
        ("echo x > .clau@(de)/settings.json", CONFIG),
        ("tee .clau?(de)/settings.json", CONFIG),
        ("tee .clau!(x)/settings.json", CONFIG),
        ("cp x +(.)claude/settings.json", CONFIG),
        ("tee notes@(a|b).md", ALLOW),
    ]);
}

#[test]
fn a_quoted_parenthesis_in_a_target_does_not_close_its_group_early() {
    assert_table(&[
        (r#"tee .c@(laude|l")")/settings.json"#, CONFIG),
        (r#"tee .c@(l")"|laude)/settings.json"#, CONFIG),
        (r#"echo x > .c@(l")"|laude)/settings.json"#, CONFIG),
        (r#"tee .c@(l")"|x)aude/settings.json"#, CONFIG),
        (r#"tee .c@(l")"|x)aude.bak/settings.json"#, ALLOW),
        (
            r#"bash -O extglob -c 'tee .c@(l")"|laude)/settings.json'"#,
            CONFIG,
        ),
    ]);
}

/// Bash matches a group within one component: an alternative holding a `/` only fails to match.
#[test]
fn a_slash_inside_a_group_does_not_split_the_path() {
    assert_table(&[
        ("tee .clau@(de|x/y)/settings.json", CONFIG),
        ("tee .anne@(al|x/y)/config.json", CONFIG),
        ("rm -r .clau@(de|x/y)", CONFIG),
        ("mv .anne@(al|x/y) /tmp/x", CONFIG),
        ("chmod -R 777 .clau@(de|x/y)", CONFIG),
        ("tee @(a)/.clau@(de|x/y)/@(s)ettings.json", CONFIG),
        ("tee @(a)/.claude/@(s)ettings.json", CONFIG),
        ("tee notes/x@(a|b/c).txt", ALLOW),
    ]);
}

/// Bash reads a special parameter's name before `(` as text, so `$@(` opens a group whose `#`
/// is no comment.
#[test]
fn a_special_parameter_before_a_parenthesis_opens_a_group() {
    assert_table(&[
        ("echo A $@(x # ) ; rm -rf /\n)", RM_RF),
        (
            "echo A $@(x # ) ; echo x > .claude/settings.json\n)",
            CONFIG,
        ),
        ("echo A $?(x # ) ; git push -f origin main\n)", FORCE_PUSH),
        ("echo $@(x)", ALLOW),
    ]);
}

#[test]
fn an_empty_group_outside_a_function_name_is_a_pattern() {
    assert_table(&[
        ("rm @()-rf /home", RM_RF),
        ("tee .clau@()de/settings.json", CONFIG),
        ("echo x > .clau@( )de/settings.json", CONFIG),
    ]);
}

/// Where a function's name may stand, `f?()` is a definition without `extglob` and one pattern
/// word with it (`/bin/echo@() { hi }` runs `echo`), so it cannot be read either way.
#[test]
fn an_opener_before_an_empty_group_where_a_name_stands_is_unreadable() {
    assert_table(&[
        ("f?() { rm -rf /tmp/x; }; f?", UNREADABLE),
        ("f@() { echo ok; }; f@", UNREADABLE),
        ("function f?() { echo ok; }; f?", UNREADABLE),
        ("f?( ) { echo ok; }; f?", UNREADABLE),
        ("f@()\n{ echo ok; }; f@", UNREADABLE),
        ("f@(\\\n) { echo ok; }; f@", UNREADABLE),
        ("/bin/rm@() -rf ~", UNREADABLE),
        ("rm@( ) -rf /", UNREADABLE),
        ("git@() push -f origin main", UNREADABLE),
        ("bash -O extglob -c '/bin/rm@() { -rf ~ }'", UNREADABLE),
        ("bash -O extglob -c '/bin/rm@() [[ -rf ~ ]]'", UNREADABLE),
        ("shopt -s extglob\n/bin/rm@() { -rf ~\n}", UNREADABLE),
        (
            "shopt -s extglob\n/bin/rm@() if -rf ~\nthen :; fi",
            UNREADABLE,
        ),
        // A name with no opener before it defines a function, as before.
        ("f() { rm -rf /tmp/x; }; f", ALLOW),
        ("f() { rm -rf /home; }", RM_RF),
    ]);
}

/// With `extglob` a group in the program word may name any program, as `?` and `*` may.
#[test]
fn a_group_in_the_program_word_is_a_pattern_program() {
    assert_table(&[
        ("/bin/r@(m) -rf ~", RM_RF),
        ("r@(m) -rf /", RM_RF),
        ("sudo /bin/r@(m) -rf ~", RM_RF),
        ("/usr/bin/gi@(t) push -f origin main", FORCE_PUSH),
        ("/bin/ec@(h)o hello", ALLOW),
    ]);
}

#[test]
fn a_bang_group_in_command_position_is_still_a_negated_subshell() {
    assert_table(&[
        ("!(rm -rf /)", RM_RF),
        ("if !(git push -f origin main); then echo x; fi", FORCE_PUSH),
        ("if !(false); then echo yes; fi", ALLOW),
        ("time -p !(rm -rf /)", RM_RF),
        ("time -- !(rm -rf /)", RM_RF),
        ("time -p -- !(git push -f origin main)", FORCE_PUSH),
        (
            "if true; then time -p !(echo x > .claude/settings.json); fi",
            CONFIG,
        ),
        ("function f { !(rm -rf /); }; f", RM_RF),
        ("coproc { !(rm -rf /); }", RM_RF),
        ("coproc c { !(rm -rf /); }", RM_RF),
        ("function f { { !(rm -rf /); }; }; f", RM_RF),
        ("coproc rm !(x) /home", RM_RF),
        ("[[ $x == @(a|b) ]] && echo y", ALLOW),
        ("[[ $x == !(a) ]] && echo y", ALLOW),
        ("case $x in @(a|b)) rm -rf /;; esac", RM_RF),
        ("case $x in @(a|b)) echo ok;; esac", ALLOW),
    ]);
}
