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

// Review round 1 of #89: each leak the reviewer saw reach Clef, in prose.

#[test]
fn a_credential_option_value_is_redacted() {
    assert_eq!(
        safe_prompt("deploy --password hunter2 now"),
        "deploy --password <redacted> now"
    );
}

#[test]
fn a_registry_login_password_option_is_redacted() {
    assert_eq!(
        safe_prompt("run docker login -u me -p hunter2 ghcr.io"),
        "run docker login -u me -p <redacted> ghcr.io"
    );
}

#[test]
fn credential_named_header_values_are_redacted() {
    assert_eq!(
        safe_prompt("send \"X-Api-Key: abc\" and \"Cookie: s=1; t=2\" or PRIVATE-TOKEN: t x now"),
        "send \"X-Api-Key: <redacted>\" and \"Cookie: <redacted>; <redacted>\" or PRIVATE-TOKEN: \
         <redacted> <redacted> now"
    );
}

#[test]
fn a_credential_key_inside_an_option_value_is_redacted() {
    assert_eq!(
        safe_prompt("run kubectl create secret generic s --from-literal=password=hunter2 now"),
        "run kubectl create secret generic s --from-literal=password=<redacted> now"
    );
}

#[test]
fn a_credential_system_property_is_redacted() {
    assert_eq!(
        safe_prompt("use mvn -Dpassword=hunter2 deploy"),
        "use mvn -Dpassword=<redacted> deploy"
    );
}

#[test]
fn single_dash_credential_options_are_redacted() {
    assert_eq!(
        safe_prompt("keytool -storepass changeit -keypass k2 -list"),
        "keytool -storepass <redacted> -keypass <redacted> -list"
    );
}

#[test]
fn an_openssl_pass_phrase_is_redacted() {
    assert_eq!(
        safe_prompt("openssl pkcs12 -passout pass:hunter2 -in c.pem, or try pass:hunter2 here"),
        "openssl pkcs12 -passout <redacted> -in c.pem, or try pass:<redacted> here"
    );
}

#[test]
fn a_github_secret_body_is_redacted() {
    assert_eq!(
        safe_prompt("gh secret set TOKEN --body hunter2 then"),
        "gh secret set TOKEN --body <redacted> then"
    );
}

#[test]
fn a_quoted_program_name_or_option_is_still_read() {
    assert_eq!(
        safe_prompt("run \"mysql\" -phunter2 or mysql '-phunter2' db"),
        "run \"mysql\" -p<redacted> or mysql -p<redacted> db"
    );
}

#[test]
fn url_userinfo_holding_a_hash_is_redacted() {
    assert_eq!(
        safe_prompt("clone https://user:p#ss@host/x now"),
        "clone https://<redacted>@host/x now"
    );
}

#[test]
fn an_ldap_bind_password_is_redacted() {
    assert_eq!(
        safe_prompt("ldapsearch -w hunter2 -b dc=x"),
        "ldapsearch -w <redacted> -b dc=x"
    );
}

#[test]
fn a_sql_server_password_is_redacted() {
    assert_eq!(
        safe_prompt("sqlcmd -U sa -P hunter2 -Q q"),
        "sqlcmd -U sa -P <redacted> -Q q"
    );
}

#[test]
fn an_npm_auth_token_is_redacted() {
    assert_eq!(
        safe_prompt("set //registry.npmjs.org/:_authToken=abc please"),
        "set //registry.npmjs.org/:_authToken=<redacted> please"
    );
}

#[test]
fn a_quoted_authorization_key_has_its_value_redacted() {
    assert_eq!(
        safe_prompt("send {\"Authorization\": \"Token abc\"} now"),
        "send {\"Authorization\": \"<redacted> now"
    );
}

#[test]
fn a_credential_in_a_json_body_is_redacted() {
    assert_eq!(
        safe_prompt("post {\"password\":\"hunter2\"} now"),
        "post {\"password\":<redacted> now"
    );
}

#[test]
fn an_smb_user_and_password_is_redacted() {
    assert_eq!(
        safe_prompt("smbclient //h/s -U me%hunter2"),
        "smbclient //h/s -U <redacted>"
    );
}

#[test]
fn a_quoted_option_value_is_redacted_through_its_close() {
    assert_eq!(
        safe_prompt("curl -u \"me:my pass\" x"),
        "curl -u \"<redacted> x"
    );
}

// Review round 2 of #89.

#[test]
fn a_credential_config_key_has_its_next_word_redacted() {
    assert_eq!(
        safe_prompt("set aws_secret_access_key X and git config user.password Y now"),
        "set aws_secret_access_key <redacted> and git config user.password <redacted> now"
    );
}

#[test]
fn twine_and_az_login_passwords_are_redacted() {
    assert_eq!(
        safe_prompt("twine upload -p pw dist and az login -u me -p pw2 now"),
        "twine upload -p <redacted> dist and az login -u me -p <redacted> now"
    );
}

#[test]
fn a_vault_token_is_redacted() {
    assert_eq!(
        safe_prompt("vault login s.abcdefgh now, or use hvs.CAESIabcdef"),
        "vault login <redacted> now, or use <redacted>"
    );
}

#[test]
fn a_credential_option_read_from_stdin_or_a_file_takes_no_value() {
    let prompt = "run docker login --password-stdin -u me ghcr.io";
    assert_eq!(safe_prompt(prompt), prompt);
}

#[test]
fn url_userinfo_holding_a_question_mark_is_redacted() {
    assert_eq!(
        safe_prompt("clone https://user:p?X@host/x now"),
        "clone https://<redacted>@host/x now"
    );
}

// Final review of #89.

#[test]
fn every_cookie_pair_is_redacted_however_it_is_written() {
    let cases = [
        (
            "set Cookie:a=x; sid=S now",
            "set Cookie:<redacted> <redacted> now",
        ),
        (
            "send \"Cookie:a=x; sid=S\" now",
            "send \"Cookie:<redacted> <redacted>\" now",
        ),
        (
            "send 'Cookie: a=x; sid=\"S\"' now",
            "send 'Cookie: <redacted>; <redacted>\"' now",
        ),
        (
            "run bash -c \"curl -H 'Cookie: a=S; sid=S' u\"",
            "run bash -c \"curl -H 'Cookie: <redacted>; <redacted>' u\"",
        ),
    ];
    for (prompt, expected) in cases {
        assert_eq!(safe_prompt(prompt), expected, "{prompt:?}");
    }
}

#[test]
fn every_cookie_pair_after_an_unquoted_header_is_redacted() {
    assert_eq!(
        safe_prompt("send Cookie: a=1; b=2; c=3; d=\"4\" now"),
        "send Cookie: <redacted>; <redacted>; <redacted>; <redacted>\" now"
    );
}

#[test]
fn the_first_word_after_a_header_semicolon_that_is_no_pair_ends_the_pairs() {
    let cases = [
        (
            "send Cookie: a=1; b=2; Secure; d=4 now",
            "send Cookie: <redacted>; <redacted>; Secure; d=4 now",
        ),
        (
            "send Cookie: a=1; b=2; c=$(rm) now",
            "send Cookie: <redacted>; <redacted>; c=$(rm) now",
        ),
    ];
    for (prompt, expected) in cases {
        assert_eq!(safe_prompt(prompt), expected, "{prompt:?}");
    }
}

#[test]
fn a_query_holding_an_at_after_a_port_is_redacted_as_userinfo() {
    // Accepted over-redaction (wi-434adf): `host:8080?to=a` reads like `user:pass?X`.
    assert_eq!(
        safe_prompt("open https://host:8080?to=a@b now"),
        "open https://<redacted>@b now"
    );
}

// Review round 1 of the #89 follow-up.

#[test]
fn an_attached_header_value_hides_only_its_own_word() {
    let cases = [
        (
            "push fix-auth:main --force now",
            "push fix-auth:<redacted> --force now",
        ),
        (
            "run docker run -v db-password:/p alpine rm -rf /p",
            "run docker run -v db-password:<redacted> alpine rm -rf /p",
        ),
        (
            "send X-Api-Key:k to u now",
            "send X-Api-Key:<redacted> to u now",
        ),
    ];
    for (prompt, expected) in cases {
        assert_eq!(safe_prompt(prompt), expected, "{prompt:?}");
    }
}

#[test]
fn a_cookie_pair_whose_value_opens_a_quote_it_does_not_close_stays_readable() {
    assert_eq!(
        safe_prompt("send Cookie: a=1; b=2; c='rm -rf ~' now"),
        "send Cookie: <redacted>; <redacted>; c='rm -rf ~' now"
    );
}
