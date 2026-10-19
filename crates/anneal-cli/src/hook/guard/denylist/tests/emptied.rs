//! A word holding an expansion is judged with its expansions empty too, pattern or not (#125):
//! with `d=` empty, `"$d".claude/settings.json` is `.claude/settings.json`.

use super::{ALLOW, CONFIG, assert_table};

#[test]
fn a_config_write_through_an_expansion_with_no_pattern_is_judged_emptied() {
    assert_table(&[
        ("tee \"$d\".claude/settings.json", CONFIG),
        ("tee -a \"${d}\".claude/settings.local.json", CONFIG),
        ("tee \"$(printf '')\".claude/settings.json", CONFIG),
        ("truncate -s 0 \"$d\".claude/settings.json", CONFIG),
        ("cp x \"$d\".claude/settings.json", CONFIG),
        ("mv x \"$d\".claude/settings.json", CONFIG),
        ("sed -i s/a/b/ \"$d\".claude/settings.json", CONFIG),
        ("rm -f \"$d\".claude/settings.json", CONFIG),
        ("chmod 644 \"$d\".claude/settings.json", CONFIG),
        ("tee $d.claude/settings.json", CONFIG),
        ("tee \"$a\".claude/\"$b\"settings.json", CONFIG),
        ("sudo tee \"$d\".claude/settings.json", CONFIG),
        ("bash -c 'tee \"$d\".claude/settings.json'", CONFIG),
    ]);
}

#[test]
fn a_redirection_through_an_expansion_with_no_pattern_is_judged_emptied() {
    assert_table(&[
        ("echo x > \"$d\".claude/settings.json", CONFIG),
        ("echo x >> \"$d\".claude/settings.json", CONFIG),
        ("echo x 2> \"$d\".claude/settings.json", CONFIG),
        ("echo x &> \"$d\".claude/settings.json", CONFIG),
        ("echo x >| \"${d}\".claude/settings.local.json", CONFIG),
        ("echo x > $d.claude/settings.json", CONFIG),
    ]);
}

#[test]
fn an_emptied_reading_that_lands_on_no_protected_path_is_allowed() {
    assert_table(&[
        ("tee \"$out\".log", ALLOW),
        ("echo x > \"$out\".log", ALLOW),
        ("echo x > \"$d\"/notes.md", ALLOW),
        ("tee \"$d\"/settings.json", ALLOW),
        ("cp \"$f\" \"$f\".bak", ALLOW),
        ("mv \"$d\".claude-old x", ALLOW),
        ("echo x > \"$HOME\"/.cache/x.json", ALLOW),
        ("tee \"$d\"claude/settings.json", ALLOW),
    ]);
}
