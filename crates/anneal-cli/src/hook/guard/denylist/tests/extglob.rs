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
fn a_bang_group_in_command_position_is_still_a_negated_subshell() {
    assert_table(&[
        ("!(rm -rf /)", RM_RF),
        ("if !(git push -f origin main); then echo x; fi", FORCE_PUSH),
        ("if !(false); then echo yes; fi", ALLOW),
        ("[[ $x == @(a|b) ]] && echo y", ALLOW),
        ("[[ $x == !(a) ]] && echo y", ALLOW),
        ("case $x in @(a|b)) rm -rf /;; esac", RM_RF),
        ("case $x in @(a|b)) echo ok;; esac", ALLOW),
    ]);
}
