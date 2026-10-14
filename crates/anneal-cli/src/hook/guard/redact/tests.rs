use super::{MAX_COMMAND_CHARS, UNPARSED, redact};

mod ambiguous;
mod probes;
mod shapes;
mod skeleton;

/// Asserts each command's skeleton.
fn assert_cases(cases: &[(&str, &str)]) {
    for &(command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn an_env_prefix_value_is_redacted() {
    assert_eq!(
        redact("FOO=bar cargo test"),
        "FOO=<redacted> cargo <redacted>"
    );
}

#[test]
fn every_prefix_of_every_command_is_redacted_whatever_its_quoting() {
    assert_cases(&[
        (
            "A=1 B='two words' C=\"x $(cat secret) y\" run",
            "cat <redacted>; A=<redacted> B=<redacted> C=<redacted> run",
        ),
        (
            "TOKEN=$(pass show ci) && X=y deploy",
            "pass <redacted> <redacted>; TOKEN=<redacted>; X=<redacted> deploy",
        ),
        (
            "ls; K=v cmd | P=q other",
            "ls; K=<redacted> cmd | P=<redacted> other",
        ),
        ("echo hi\nK=v cmd", "echo <redacted>; K=<redacted> cmd"),
        ("(K=v cmd)", "K=<redacted> cmd"),
        ("K=a\\ b cmd", "K=<redacted> cmd"),
        ("K=$'a b' cmd", "K=<redacted> cmd"),
        ("K=`cat f` cmd", "cat <redacted>; K=<redacted> cmd"),
        ("K=${HOME}/x cmd", "K=<redacted> cmd"),
        (
            "if K=v cmd; then L=w x; fi",
            "if K=<redacted> cmd; then L=<redacted> x; fi",
        ),
    ]);
}

#[test]
fn export_env_and_the_declaration_builtins_have_their_values_redacted() {
    assert_cases(&[
        ("export API_KEY=abc", "export <redacted>"),
        ("export A=1 B=2", "export <redacted> <redacted>"),
        ("env -i A=b cmd x=y", "env -i A=<redacted> cmd <redacted>"),
        ("env A=b B=c cmd", "env A=<redacted> B=<redacted> cmd"),
        ("declare -x A=b", "declare -x <redacted>"),
        ("local A=b", "local <redacted>"),
        ("readonly A=b", "readonly <redacted>"),
        ("typeset A=b", "typeset <redacted>"),
    ]);
}

#[test]
fn an_operand_assignment_shows_only_a_dd_key_and_a_sensitive_target() {
    assert_cases(&[
        (
            "dd if=/dev/zero of=/dev/sda bs=1M",
            "dd if=/dev/zero of=/dev/sda <redacted>",
        ),
        ("git config user.name=x", "git <redacted> <redacted>"),
        ("find . -name x=y", "find . -name <redacted>"),
    ]);
}

#[test]
fn an_env_style_assignment_is_redacted_in_any_position() {
    assert_cases(&[
        ("make CC=gcc", "make <redacted>"),
        ("echo A_1=b", "echo <redacted>"),
        ("git commit -m \"FIX=b\"", "git commit -m <redacted>"),
        ("sudo K=v", "sudo K=<redacted>"),
        ("sudo env K=v", "sudo env K=<redacted>"),
        ("timeout 5 env K=v", "timeout <redacted> env K=<redacted>"),
        ("command export K=v", "command export <redacted>"),
        ("env -u NAME K=v", "env -u <redacted> K=<redacted>"),
        // nohup runs `K=v` as its program, which the denylist reads as an assignment.
        ("nohup K=v run", "nohup <redacted> <redacted>"),
    ]);
}

#[test]
fn a_shell_body_in_quotes_is_scanned_as_text() {
    assert_cases(&[
        ("bash -c 'K=v cmd'", "bash -c 'K=<redacted> cmd'"),
        ("eval \"K=v ls\"", "eval 'K=<redacted> ls'"),
        ("sh -c \"$(echo K=v)\"", "echo <redacted>; sh -c <redacted>"),
    ]);
}

#[test]
fn a_name_holding_a_credential_word_is_redacted_in_any_letter_case() {
    assert_cases(&[
        ("deploy api_token=x", "deploy <redacted>"),
        ("run Db_Password=p go", "run <redacted> <redacted>"),
    ]);
}

#[test]
fn array_and_element_assignments_are_redacted() {
    assert_cases(&[
        // The guard's lexer reads `K=(` as the word `K=` and a subshell.
        ("K=(a b) cmd", "K=<redacted>; a <redacted>; cmd"),
        ("K[0]=v cmd", "<redacted> <redacted>"),
        ("K+=v cmd", "<redacted> <redacted>"),
    ]);
}

#[test]
fn a_comment_is_redacted_to_the_end_of_its_line() {
    assert_eq!(
        redact("# let's deploy\nAPI_KEY=sk-live-123 ./deploy.sh"),
        "API_KEY=<redacted> deploy.sh"
    );
    assert_eq!(redact("ls # TOKEN is x"), "ls");
}

#[test]
fn ansi_c_quoting_is_not_read_inside_double_quotes() {
    assert_eq!(
        redact("echo \"$'\" ; TOKEN=abc deploy"),
        "echo <redacted>; TOKEN=<redacted> deploy"
    );
}

#[test]
fn a_heredoc_body_is_redacted_entirely() {
    assert_cases(&[
        (
            "cat <<EOF\nit's\nEOF\nTOKEN=x run",
            "cat <<< <redacted>; TOKEN=<redacted> run",
        ),
        (
            "cat <<-'END' > f\n\tsecret: x\n\tEND\nls",
            "cat > <redacted> <<< <redacted>; ls",
        ),
        ("cat <<<\"TOKEN=x\"", "cat <<< <redacted>"),
    ]);
}

#[test]
fn an_unclosed_span_anywhere_redacts_the_rest_of_the_command() {
    assert_cases(&[
        ("echo 'abc TOKEN=x", UNPARSED),
        ("ls $(echo TOKEN=x", UNPARSED),
        ("echo \"a b", UNPARSED),
        // The lexer reads an unclosed heredoc's body to the end of the command.
        ("cat <<EOF\nTOKEN=x", "cat <<< <redacted>"),
    ]);
}

#[test]
fn values_of_credential_flags_are_redacted() {
    assert_cases(&[
        (
            "curl --token=abc https://x",
            "curl --token=<redacted> <redacted>",
        ),
        (
            "login --password hunter2 --user me",
            "login --password <redacted> --user <redacted>",
        ),
        (
            "tool --api-key='k k' go",
            "tool --api-key=<redacted> <redacted>",
        ),
        ("tool --API_KEY=k", "tool --API_KEY=<redacted>"),
        (
            "vault --client-secret s",
            "vault --client-secret <redacted>",
        ),
        ("gh --auth-token=t", "gh --auth-token=<redacted>"),
        ("tool --passwd p", "tool --passwd <redacted>"),
    ]);
}

#[test]
fn flags_that_only_resemble_credentials_keep_their_names_and_hide_their_values() {
    assert_cases(&[
        (
            "git commit --author='A <a@b>' -m x",
            "git commit --author=<redacted> -m <redacted>",
        ),
        ("sort --key=2", "sort --key=<redacted>"),
        ("tool --password", "tool --password"),
        ("tool --password; ls", "tool --password; ls"),
    ]);
}

#[test]
fn the_command_is_trimmed() {
    assert_eq!(redact("  \n ls -la \t\n"), "ls -<redacted>");
}

#[test]
fn a_long_command_is_cut_at_a_char_boundary_and_marked() {
    let command = "ls; ".repeat(MAX_COMMAND_CHARS);

    let redacted = redact(&command);

    assert_eq!(
        redacted.chars().count(),
        MAX_COMMAND_CHARS + 1,
        "cut, plus the ellipsis"
    );
    assert!(redacted.starts_with("ls; ls"), "{redacted}");
    assert!(redacted.ends_with('…'), "marked as cut");
}

#[test]
fn values_are_redacted_before_the_cut_so_a_long_secret_does_not_crowd_out_the_command() {
    let command = format!("SECRET={} rm -r build", "h".repeat(2 * MAX_COMMAND_CHARS));

    assert_eq!(redact(&command), "SECRET=<redacted> rm -r <redacted>");
}

#[test]
fn an_unclosed_quote_redacts_to_the_end() {
    let redacted = redact("K='abc def");

    assert_eq!(redacted, UNPARSED);
    assert!(!redacted.contains("abc"), "{redacted}");
}

#[test]
fn a_quoted_assignment_name_has_its_value_redacted() {
    assert_cases(&[
        ("export \"API_KEY\"=x", "export <redacted>"),
        ("declare -x 'PASSWORD'=x", "declare -x <redacted>"),
        (
            "export \"db_\"'token'=x run",
            "export <redacted> <redacted>",
        ),
    ]);
}

#[test]
fn the_words_pass_auth_and_bearer_name_credentials() {
    assert_cases(&[
        ("deploy db_pass=x", "deploy <redacted>"),
        ("run my_auth=x go", "run <redacted> <redacted>"),
        (
            "tool --bearer t --pass p",
            "tool --bearer <redacted> --pass <redacted>",
        ),
        ("gh --oauth=t", "gh --oauth=<redacted>"),
        ("tool --authorization x", "tool --authorization <redacted>"),
    ]);
}

#[test]
fn authorization_header_values_are_redacted() {
    assert_cases(&[
        (
            "curl -H \"Authorization: Bearer abc\" https://x",
            "curl -H <redacted> <redacted>",
        ),
        (
            "curl -H 'proxy-authorization: token t' -X GET u",
            "curl -H <redacted> -X <redacted> <redacted>",
        ),
        (
            "curl -HAuthorization:Basic\\ abc u; ls",
            "curl -<redacted> <redacted>; ls",
        ),
    ]);
}

#[test]
fn the_credential_after_bearer_or_basic_is_redacted() {
    assert_cases(&[
        (
            "echo Bearer abc def",
            "echo <redacted> <redacted> <redacted>",
        ),
        ("printf 'basic dXNlcjpwYXNz'", "printf <redacted>"),
        ("echo BASIC Abc", "echo <redacted> <redacted>"),
    ]);
}

#[test]
fn a_plain_lowercase_word_after_basic_is_hidden_like_any_argument() {
    assert_eq!(redact("echo basic usage"), "echo <redacted> <redacted>");
}

#[test]
fn url_userinfo_is_redacted() {
    assert_cases(&[
        (
            "git clone https://user:tok@github.com/o/r.git",
            "git <redacted> <redacted>",
        ),
        ("curl \"https://ci:abc@host/p?q=1\"", "curl <redacted>"),
        (
            "tool --url=ftp://tok@h,ssh://u:p@k/x",
            "tool --url=<redacted>",
        ),
    ]);
}

#[test]
fn urls_without_userinfo_are_hidden_like_any_argument() {
    assert_cases(&[
        (
            "git clone git@github.com:o/r.git",
            "git <redacted> <redacted>",
        ),
        ("curl https://host/a@b", "curl <redacted>"),
        ("open https://host?to=a@b", "open <redacted>"),
    ]);
}

#[test]
fn secret_short_options_of_known_tools_are_redacted() {
    assert_cases(&[
        (
            "mysql -uroot -phunter2 db",
            "mysql -<redacted> -<redacted> <redacted>",
        ),
        (
            "sudo /usr/bin/mysqldump -p'p w' db",
            "sudo mysqldump -<redacted> <redacted>",
        ),
        (
            "curl -u user:pass https://x",
            "curl -u <redacted> <redacted>",
        ),
        (
            "curl -su user:pass x",
            "curl -<redacted> <redacted> <redacted>",
        ),
        ("curl -uuser:pass x", "curl -<redacted> <redacted>"),
        (
            "curl --user user:pass x",
            "curl --user <redacted> <redacted>",
        ),
        (
            "curl --proxy-user=u:p x",
            "curl --proxy-user=<redacted> <redacted>",
        ),
        (
            "sshpass -p pw ssh h",
            "sshpass -p <redacted> <redacted> <redacted>",
        ),
        ("redis-cli -a pw ping", "redis-cli -a <redacted> <redacted>"),
        ("mongosh -u me -p pw", "mongosh -u <redacted> -p <redacted>"),
        ("bash -c 'mysql -pX'", "bash -c 'mysql -<redacted>'"),
    ]);
}

#[test]
fn short_options_of_other_tools_are_kept() {
    assert_cases(&[
        ("mkdir -p a/b", "mkdir -p <redacted>"),
        (
            "docker run -p 8080:80 img",
            "docker <redacted> -p <redacted> <redacted>",
        ),
        ("mysql -p db", "mysql -p <redacted>"),
        ("curl x; ls -u", "curl <redacted>; ls -u"),
        ("ssh -p 22 host", "ssh -p <redacted> <redacted>"),
        ("git add -p", "git <redacted> -p"),
    ]);
}

#[test]
fn a_quote_opened_in_the_middle_of_a_value_is_redacted_through_its_close() {
    assert_eq!(redact("TOKEN=abc\"def ghi\" run"), "TOKEN=<redacted> run");
}

// Review round 1 of #89: each leak the reviewer saw reach the log in shadow mode.

#[test]
fn a_registry_login_password_option_is_redacted() {
    assert_cases(&[
        (
            "docker login -u me -p hunter2 ghcr.io",
            "docker <redacted> -u <redacted> -p <redacted> <redacted>",
        ),
        (
            "podman login -phunter2 quay.io",
            "podman <redacted> -<redacted> <redacted>",
        ),
        (
            "helm registry login h -p pw",
            "helm <redacted> <redacted> <redacted> -p <redacted>",
        ),
    ]);
}

#[test]
fn credential_named_header_values_are_redacted() {
    assert_cases(&[
        (
            "curl -H \"X-Api-Key: abc\" -H 'PRIVATE-TOKEN: t' u",
            "curl -H <redacted> -H <redacted> <redacted>",
        ),
        (
            "curl -H \"Cookie: s=1; t=2\" u",
            "curl -H <redacted> <redacted>",
        ),
    ]);
}

#[test]
fn a_credential_key_inside_an_option_value_is_redacted() {
    assert_eq!(
        redact(
            "kubectl create secret generic s --from-literal=password=hunter2 --from-literal=user=me"
        ),
        "kubectl <redacted> secret <redacted> <redacted> --from-literal=<redacted> \
         --from-literal=<redacted>"
    );
}

#[test]
fn a_credential_system_property_is_redacted() {
    assert_eq!(
        redact("mvn deploy -Dpassword=hunter2 -Ddb.password=pw -Dskip=true"),
        "mvn <redacted> -<redacted> -<redacted> -<redacted>"
    );
}

#[test]
fn single_dash_credential_options_are_redacted() {
    assert_eq!(
        redact("keytool -importcert -storepass changeit -keypass k2 -alias a"),
        "keytool -<redacted> -<redacted> <redacted> -<redacted> <redacted> -<redacted> <redacted>"
    );
}

#[test]
fn an_openssl_pass_phrase_is_redacted() {
    assert_cases(&[
        (
            "openssl pkcs12 -export -passout pass:hunter2 -in c.pem",
            "openssl <redacted> -<redacted> -<redacted> <redacted> -<redacted> <redacted>",
        ),
        (
            "openssl enc -k pass:hunter2 -in f",
            "openssl <redacted> -k <redacted> -<redacted> <redacted>",
        ),
    ]);
}

#[test]
fn a_github_secret_body_and_any_other_body_are_redacted() {
    assert_cases(&[
        (
            "gh secret set TOKEN --body hunter2",
            "gh secret <redacted> <redacted> --body <redacted>",
        ),
        (
            "gh secret set K -bhunter2",
            "gh secret <redacted> <redacted> -<redacted>",
        ),
        (
            "gh pr create --body text",
            "gh pr <redacted> --body <redacted>",
        ),
    ]);
}

#[test]
fn a_quoted_or_escaped_program_name_is_still_a_known_tool() {
    assert_cases(&[
        ("\"mysql\" -phunter2", "mysql -<redacted>"),
        ("\\mysql -phunter2", "mysql -<redacted>"),
        ("mysql '-phunter2' db", "mysql -<redacted> <redacted>"),
    ]);
}

#[test]
fn url_userinfo_holding_a_hash_is_redacted() {
    assert_eq!(redact("curl https://user:p#ss@host/x"), "curl <redacted>");
}

#[test]
fn an_ldap_bind_password_is_redacted() {
    assert_eq!(
        redact("ldapsearch -x -D cn=admin -w hunter2 -b dc=x"),
        "ldapsearch -x -D <redacted> -w <redacted> -b <redacted>"
    );
}

#[test]
fn a_sql_server_password_is_redacted() {
    assert_eq!(
        redact("sqlcmd -S db -U sa -P hunter2 -Q q"),
        "sqlcmd -S <redacted> -U <redacted> -P <redacted> -Q <redacted>"
    );
}

#[test]
fn an_npm_auth_token_is_redacted() {
    assert_eq!(
        redact("npm config set //registry.npmjs.org/:_authToken=abc"),
        "npm <redacted> <redacted> <redacted>"
    );
}

#[test]
fn a_quoted_authorization_key_has_its_value_redacted() {
    assert_eq!(
        redact("curl -d '{\"Authorization\": \"Token abc\"}' u"),
        "curl -d <redacted> <redacted>"
    );
}

#[test]
fn a_credential_in_a_json_body_is_redacted() {
    assert_cases(&[
        (
            "curl -d '{\"password\":\"hunter2\"}' u",
            "curl -d <redacted> <redacted>",
        ),
        (
            "curl -d '{\"password\": \"hunter2\", \"user\": \"me\"}' u",
            "curl -d <redacted> <redacted>",
        ),
    ]);
}

#[test]
fn an_smb_user_and_password_is_redacted() {
    assert_eq!(
        redact("smbclient //h/s -U me%hunter2"),
        "smbclient <redacted> -U <redacted>"
    );
}

#[test]
fn an_obvious_token_is_redacted_in_a_command() {
    assert_eq!(
        redact("echo ghp_0123456789abcdefghij | gh auth login"),
        "echo <redacted> | gh <redacted> <redacted>"
    );
}

#[test]
fn a_credential_key_with_its_value_in_the_word_hides_only_that_word() {
    assert_eq!(
        redact("git push origin auth-fix:main --force"),
        "git push origin <redacted> --force"
    );
}

#[test]
fn a_word_holding_a_path_separator_is_hidden_like_any_argument() {
    assert_eq!(
        redact("cargo test auth::tests::login"),
        "cargo <redacted> <redacted>"
    );
}

// Review round 2 of #89.

#[test]
fn a_header_in_a_shell_body_does_not_hide_the_next_command() {
    assert_cases(&[
        (
            "bash -c 'curl -H X-Api-Key: k; dropdb production'",
            "bash -c 'curl -H <redacted> <redacted>; dropdb <redacted>'",
        ),
        (
            "x=$(echo token: a; kubectl delete ns prod)",
            "echo <redacted> <redacted>; kubectl delete ns <redacted>; x=<redacted>",
        ),
    ]);
}

#[test]
fn cookie_pairs_after_a_semicolon_in_a_quote_stay_hidden_but_a_command_does_not() {
    assert_cases(&[
        (
            "curl -H 'Cookie: a=1; b=2; c=3' u",
            "curl -H <redacted> <redacted>",
        ),
        (
            "bash -c 'curl -H Cookie: a=1; b=2 rm -rf /tmp/x'",
            "bash -c 'curl -H <redacted> <redacted>; b=<redacted> rm -rf <redacted>'",
        ),
        (
            "bash -c 'curl -H Cookie: a=1 | b=2 sh'",
            "bash -c 'curl -H <redacted> <redacted> | b=<redacted> sh'",
        ),
    ]);
}

#[test]
fn no_command_after_a_separator_is_hidden_by_an_earlier_header() {
    // TST-06 by enumeration: every header form, before every separator, in every nesting.
    let headers = [
        "echo token:",
        "echo token: a",
        "curl -H X-Api-Key: k",
        "curl -H Authorization: Bearer x",
        "curl -H Cookie: a=1",
        "printf %s Cookie:",
        "curl -H Cookie:a=1",
        "curl -H Cookie: a=1; b=2",
        "curl -H X-Api-Key:k",
        "docker run -v db-password:/p alpine",
        "git push origin fix-auth:main --force",
    ];
    let separators = [";", " ;", "&&", "||", "|", "&", "\n"];
    let nests: [(&str, &str); 6] = [
        ("", ""),
        ("bash -c '", "'"),
        ("bash -c \"", "\""),
        ("x=$(", ")"),
        ("x=`", "`"),
        ("eval \"", "\""),
    ];
    for header in headers {
        for separator in separators {
            for (open, close) in nests {
                let command = format!("{open}{header}{separator} dropdb production{close}");

                let redacted = redact(&command);

                assert!(
                    redacted.contains("dropdb") && !redacted.contains("production"),
                    "{command:?} became {redacted:?}"
                );
            }
        }
    }
}

#[test]
fn a_credential_config_key_has_its_next_word_redacted() {
    assert_cases(&[
        (
            "aws configure set aws_secret_access_key X",
            "aws <redacted> <redacted> <redacted> <redacted>",
        ),
        (
            "git config user.password X",
            "git <redacted> <redacted> <redacted>",
        ),
    ]);
}

#[test]
fn twine_and_az_login_passwords_are_redacted() {
    assert_cases(&[
        (
            "twine upload -u me -p pw dist/x",
            "twine <redacted> -u <redacted> -p <redacted> <redacted>",
        ),
        (
            "az login -u me -p pw",
            "az <redacted> -u <redacted> -p <redacted>",
        ),
    ]);
}

#[test]
fn a_vault_token_is_redacted() {
    assert_cases(&[
        ("vault login s.abcdefgh", "vault <redacted> <redacted>"),
        ("echo hvs.CAESIabcdef", "echo <redacted>"),
    ]);
}

#[test]
fn a_credential_option_read_from_stdin_or_a_file_takes_no_value() {
    assert_eq!(
        redact("docker login --password-stdin -u me ghcr.io"),
        "docker <redacted> --password-stdin -u <redacted> <redacted>"
    );
}

#[test]
fn url_userinfo_holding_a_question_mark_is_redacted() {
    assert_eq!(redact("curl https://user:p?X@host/x"), "curl <redacted>");
}

// Final review of #89.

#[test]
fn a_cookie_with_no_blank_after_its_colon_hides_every_pair() {
    assert_eq!(
        redact("curl -H \"Cookie:a=x; sid=S\" u"),
        "curl -H <redacted> <redacted>"
    );
}

#[test]
fn a_cookie_pair_with_a_quoted_value_is_hidden() {
    assert_eq!(
        redact("curl -H 'Cookie: a=x; sid=\"S\"' u"),
        "curl -H <redacted> <redacted>"
    );
}

#[test]
fn a_cookie_pair_ending_in_the_quote_that_closes_its_header_is_hidden() {
    assert_eq!(
        redact("bash -c \"curl -H 'Cookie: a=S; sid=S' u\""),
        "bash -c 'curl -H <redacted> <redacted>'"
    );
}

#[test]
fn a_substitution_after_a_cookie_semicolon_shows_its_command_and_hides_the_word() {
    assert_eq!(
        redact("bash -c 'curl -H Cookie: a=1; b=$(dropdb production)'"),
        "bash -c 'curl -H <redacted> <redacted>; dropdb <redacted>; b=<redacted>'"
    );
}

#[test]
fn a_word_after_a_cookie_semicolon_that_is_no_pair_ends_the_hidden_pairs() {
    assert_eq!(
        redact("curl -H 'Cookie: a=1; Secure; b=2' u"),
        "curl -H <redacted> <redacted>"
    );
}

#[test]
fn a_query_holding_an_at_after_a_port_is_redacted_as_userinfo() {
    assert_eq!(redact("open https://host:8080?to=a@b"), "open <redacted>");
}

// Review round 1 of the #89 follow-up.

#[test]
fn an_attached_header_value_is_hidden_by_the_argument_rule_and_hides_nothing_more() {
    assert_cases(&[
        (
            "docker run -v db-password:/p alpine rm -rf /p",
            "docker <redacted> -v <redacted> <redacted> rm -<redacted> <redacted>",
        ),
        (
            "git push origin fix-auth:main --force",
            "git push origin <redacted> --force",
        ),
        ("curl -H X-Api-Key:k u", "curl -H <redacted> <redacted>"),
        (
            "bash -c 'curl -H Cookie:a=1 b=2 rm -rf /p'",
            "bash -c 'curl -H <redacted> <redacted> <redacted> -<redacted> <redacted>'",
        ),
    ]);
}

#[test]
fn a_cookie_pair_whose_value_holds_a_blank_or_semicolon_shows_only_its_name() {
    assert_cases(&[
        (
            "bash -c 'curl -H Cookie: a=1; c=\"rm -rf ~\"'",
            "bash -c 'curl -H <redacted> <redacted>; c=<redacted>'",
        ),
        // The body's last quote never closes: bash would refuse it too.
        (
            "bash -c \"curl -H 'Cookie: a=1; c='rm -rf ~'\"",
            "bash -c '<unparsed>'",
        ),
        (
            "bash -c 'curl -H Cookie: a=1; c=\"x;rm -rf ~\"'",
            "bash -c 'curl -H <redacted> <redacted>; c=<redacted>'",
        ),
    ]);
}

#[test]
fn a_quoted_cookie_pair_value_is_hidden_and_a_command_after_the_header_is_not() {
    assert_cases(&[
        ("curl -H 'Cookie: a=1; b=\"2\"'", "curl -H <redacted>"),
        (
            "bash -c \"curl -H 'Cookie: a=1; sid=S';dropdb production\"",
            "bash -c 'curl -H <redacted>; dropdb <redacted>'",
        ),
    ]);
}
