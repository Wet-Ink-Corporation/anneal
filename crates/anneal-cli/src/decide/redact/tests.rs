//! The prose rules added for #89 and #91. The original prose tests are in `hook::prompt::tests`.

use super::safe_prompt;

#[test]
fn a_quoted_assignment_name_has_its_value_redacted() {
    assert_eq!(
        safe_prompt("run export \"API_KEY\"=x then declare -x 'PASSWORD'=y and go"),
        "run export \"API_KEY\"=<redacted> then declare -x 'PASSWORD'=<redacted> and go"
    );
}

#[test]
fn the_words_pass_auth_and_bearer_name_credentials() {
    assert_eq!(
        safe_prompt("set db_pass=x and my_auth=y and bearer_id=z but keep author=me"),
        "set db_pass=<redacted> and my_auth=<redacted> and bearer_id=<redacted> but keep \
         author=me"
    );
}

#[test]
fn authorization_header_values_are_redacted() {
    assert_eq!(
        safe_prompt("send \"Authorization: Bearer abc\" please, or authorization:token x y"),
        "send \"Authorization: <redacted> <redacted>\" please, or authorization:<redacted> \
         <redacted> <redacted>"
    );
}

#[test]
fn the_credential_after_basic_is_redacted_but_a_plain_word_is_kept() {
    assert_eq!(
        safe_prompt("send Basic dXNlcjpwYXNz now, and add basic tests"),
        "send Basic <redacted> now, and add basic tests"
    );
}

#[test]
fn url_userinfo_is_redacted() {
    assert_eq!(
        safe_prompt("clone <https://me:tok@github.com/o/r> but keep git@github.com:o/r"),
        "clone <https://<redacted>@github.com/o/r> but keep git@github.com:o/r"
    );
}

#[test]
fn secret_short_options_of_known_tools_are_redacted() {
    assert_eq!(
        safe_prompt("run mysql -uroot -phunter2 db, curl -u me:pw x and mkdir -p dir"),
        "run mysql -uroot -p<redacted> db, curl -u <redacted> x and mkdir -p dir"
    );
}

#[test]
fn a_value_quoted_from_before_its_name_is_redacted_through_its_close() {
    assert_eq!(
        safe_prompt("use \"API_KEY=a b\" now"),
        "use \"API_KEY=<redacted> now"
    );
}

#[test]
fn a_quote_opened_in_the_middle_of_a_value_is_redacted_through_its_close() {
    assert_eq!(
        safe_prompt("set TOKEN=abc\"def ghi\" then go"),
        "set TOKEN=<redacted> then go"
    );
    assert_eq!(
        safe_prompt("set TOKEN=it's all of this"),
        "set TOKEN=<redacted>",
        "a quote never closed takes the rest"
    );
}
