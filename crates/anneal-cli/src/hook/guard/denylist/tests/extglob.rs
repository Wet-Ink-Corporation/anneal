//! Extended globs (`shopt -s extglob`, #128): `@( )`, `+( )`, `!( )`, `?( )` and `*( )` are
//! read as globs that may match a file named `-rf`, or a protected path.

use super::{ALLOW, CONFIG, FORCE_PUSH, RM_RF, assert_table};

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

#[test]
fn an_empty_group_outside_a_function_name_is_a_pattern() {
    assert_table(&[
        ("rm @()-rf /home", RM_RF),
        ("tee .clau@()de/settings.json", CONFIG),
        ("echo x > .clau@( )de/settings.json", CONFIG),
    ]);
}

#[test]
fn a_function_named_with_an_opener_character_is_still_a_definition() {
    assert_table(&[
        ("f?() { rm -rf /tmp/x; }; f?", ALLOW),
        ("f@() { echo ok; }; f@", ALLOW),
        ("f*() { rm -rf /home; }", RM_RF),
        ("f+() { git push -f origin main; }", FORCE_PUSH),
        ("function f?() { echo ok; }; f?", ALLOW),
        ("function f*() { rm -rf /home; }", RM_RF),
        ("f?( ) { echo ok; }; f?", ALLOW),
        ("f?( ) { rm -rf /home; }", RM_RF),
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
        ("[[ $x == @(a|b) ]] && echo y", ALLOW),
        ("[[ $x == !(a) ]] && echo y", ALLOW),
        ("case $x in @(a|b)) rm -rf /;; esac", RM_RF),
        ("case $x in @(a|b)) echo ok;; esac", ALLOW),
    ]);
}
