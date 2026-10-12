use super::{MAX_COMMAND_CHARS, REDACTED, redact};

#[test]
fn an_env_prefix_value_is_redacted() {
    assert_eq!(redact("FOO=bar cargo test"), "FOO=<redacted> cargo test");
}

#[test]
fn every_prefix_of_every_command_is_redacted_whatever_its_quoting() {
    let cases = [
        (
            "A=1 B='two words' C=\"x $(cat secret) y\" run",
            "A=<redacted> B=<redacted> C=<redacted> run",
        ),
        (
            "TOKEN=$(pass show ci) && X=y deploy",
            "TOKEN=<redacted> && X=<redacted> deploy",
        ),
        (
            "ls; K=v cmd | P=q other",
            "ls; K=<redacted> cmd | P=<redacted> other",
        ),
        ("echo hi\nK=v cmd", "echo hi\nK=<redacted> cmd"),
        ("(K=v cmd)", "(K=<redacted> cmd)"),
        ("K=a\\ b cmd", "K=<redacted> cmd"),
        ("K=$'a b' cmd", "K=<redacted> cmd"),
        ("K=`cat f` cmd", "K=<redacted> cmd"),
        ("K=${HOME}/x cmd", "K=<redacted> cmd"),
        (
            "if K=v cmd; then L=w x; fi",
            "if K=<redacted> cmd; then L=<redacted> x; fi",
        ),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn export_env_and_the_declaration_builtins_have_their_values_redacted() {
    let cases = [
        ("export API_KEY=abc", "export API_KEY=<redacted>"),
        ("export A=1 B=2", "export A=<redacted> B=<redacted>"),
        ("env -i A=b cmd x=y", "env -i A=<redacted> cmd x=y"),
        ("env A=b B=c cmd", "env A=<redacted> B=<redacted> cmd"),
        ("declare -x A=b", "declare -x A=<redacted>"),
        ("local A=b", "local A=<redacted>"),
        ("readonly A=b", "readonly A=<redacted>"),
        ("typeset A=b", "typeset A=<redacted>"),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn lowercase_operand_assignments_are_kept() {
    let cases = [
        "dd if=/dev/zero of=/dev/sda bs=1M",
        "git config user.name=x",
        "find . -name x=y",
    ];
    for command in cases {
        assert_eq!(redact(command), command, "{command:?}");
    }
}

#[test]
fn an_env_style_assignment_is_redacted_in_any_position() {
    let cases = [
        ("make CC=gcc", "make CC=<redacted>"),
        ("echo A_1=b", "echo A_1=<redacted>"),
        (
            "git commit -m \"FIX=b\"",
            "git commit -m \"FIX=<redacted>\"",
        ),
        ("sudo K=v", "sudo K=<redacted>"),
        ("sudo env K=v", "sudo env K=<redacted>"),
        ("timeout 5 env K=v", "timeout 5 env K=<redacted>"),
        ("command export K=v", "command export K=<redacted>"),
        ("env -u NAME K=v", "env -u NAME K=<redacted>"),
        ("nohup K=v run", "nohup K=<redacted> run"),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn a_shell_body_in_quotes_is_scanned_as_text() {
    let cases = [
        ("bash -c 'K=v cmd'", "bash -c 'K=<redacted> cmd'"),
        ("eval \"K=v ls\"", "eval \"K=<redacted> ls\""),
        ("sh -c \"$(echo K=v)\"", "sh -c \"$(echo K=<redacted>)\""),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn a_name_holding_a_credential_word_is_redacted_in_any_letter_case() {
    let cases = [
        ("deploy api_token=x", "deploy api_token=<redacted>"),
        ("run Db_Password=p go", "run Db_Password=<redacted> go"),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn array_and_element_assignments_are_redacted() {
    let cases = [
        ("K=(a b) cmd", "K=<redacted> cmd"),
        ("K[0]=v cmd", "K[0]=<redacted> cmd"),
        ("K+=v cmd", "K+=<redacted> cmd"),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn a_comment_is_redacted_to_the_end_of_its_line() {
    assert_eq!(
        redact("# let's deploy\nAPI_KEY=sk-live-123 ./deploy.sh"),
        "#<redacted>\nAPI_KEY=<redacted> ./deploy.sh"
    );
    assert_eq!(redact("ls # TOKEN is x"), "ls #<redacted>");
}

#[test]
fn ansi_c_quoting_is_not_read_inside_double_quotes() {
    assert_eq!(
        redact("echo \"$'\" ; TOKEN=abc deploy"),
        "echo \"$'\" ; TOKEN=<redacted> deploy"
    );
}

#[test]
fn a_heredoc_body_is_redacted_entirely() {
    let cases = [
        (
            "cat <<EOF\nit's\nEOF\nTOKEN=x run",
            "cat <<EOF\n<redacted>\nEOF\nTOKEN=<redacted> run",
        ),
        (
            "cat <<-'END' > f\n\tsecret: x\n\tEND\nls",
            "cat <<-'END' > f\n<redacted>\n\tEND\nls",
        ),
        ("cat <<<\"TOKEN=x\"", "cat <<<\"TOKEN=<redacted>\""),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn an_unclosed_span_anywhere_redacts_the_rest_of_the_command() {
    let cases = [
        ("echo 'abc TOKEN=x", "echo <redacted>"),
        ("ls $(echo TOKEN=x", "ls <redacted>"),
        ("echo \"a b", "echo <redacted>"),
        ("cat <<EOF\nTOKEN=x", "cat <<EOF\n<redacted>"),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn values_of_credential_flags_are_redacted() {
    let cases = [
        (
            "curl --token=abc https://x",
            "curl --token=<redacted> https://x",
        ),
        (
            "login --password hunter2 --user me",
            "login --password <redacted> --user me",
        ),
        ("tool --api-key='k k' go", "tool --api-key=<redacted> go"),
        ("tool --API_KEY=k", "tool --API_KEY=<redacted>"),
        (
            "vault --client-secret s",
            "vault --client-secret <redacted>",
        ),
        ("gh --auth-token=t", "gh --auth-token=<redacted>"),
        ("tool --passwd p", "tool --passwd <redacted>"),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn flags_that_only_resemble_credentials_are_kept() {
    let cases = [
        "git commit --author='A <a@b>' -m x",
        "sort --key=2",
        "tool --password",
        "tool --password; ls",
    ];
    for command in cases {
        assert_eq!(redact(command), command, "{command:?}");
    }
}

#[test]
fn the_command_is_trimmed() {
    assert_eq!(redact("  \n ls -la \t\n"), "ls -la");
}

#[test]
fn a_long_command_is_cut_at_a_char_boundary_and_marked() {
    let command = format!("echo {}", "é".repeat(MAX_COMMAND_CHARS));

    let redacted = redact(&command);

    assert_eq!(
        redacted.chars().count(),
        MAX_COMMAND_CHARS + 1,
        "cut, plus the ellipsis"
    );
    assert!(redacted.starts_with("echo é"), "{redacted}");
    assert!(redacted.ends_with('…'), "marked as cut");
}

#[test]
fn values_are_redacted_before_the_cut_so_a_long_secret_does_not_crowd_out_the_command() {
    let command = format!("SECRET={} rm -r build", "h".repeat(2 * MAX_COMMAND_CHARS));

    assert_eq!(redact(&command), "SECRET=<redacted> rm -r build");
}

#[test]
fn an_unclosed_quote_redacts_to_the_end() {
    assert_eq!(redact("K='abc def"), format!("K={REDACTED}"));
}

#[test]
fn a_quoted_assignment_name_has_its_value_redacted() {
    let cases = [
        ("export \"API_KEY\"=x", "export \"API_KEY\"=<redacted>"),
        (
            "declare -x 'PASSWORD'=x",
            "declare -x 'PASSWORD'=<redacted>",
        ),
        (
            "export \"db_\"'token'=x run",
            "export \"db_\"'token'=<redacted> run",
        ),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn the_words_pass_auth_and_bearer_name_credentials() {
    let cases = [
        ("deploy db_pass=x", "deploy db_pass=<redacted>"),
        ("run my_auth=x go", "run my_auth=<redacted> go"),
        (
            "tool --bearer t --pass p",
            "tool --bearer <redacted> --pass <redacted>",
        ),
        ("gh --oauth=t", "gh --oauth=<redacted>"),
        ("tool --authorization x", "tool --authorization <redacted>"),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn authorization_header_values_are_redacted() {
    let cases = [
        (
            "curl -H \"Authorization: Bearer abc\" https://x",
            "curl -H \"Authorization: <redacted> <redacted>\" https://x",
        ),
        (
            "curl -H 'proxy-authorization: token t' -X GET u",
            "curl -H 'proxy-authorization: <redacted> <redacted>' -X GET u",
        ),
        (
            "curl -HAuthorization:Basic\\ abc u; ls",
            "curl -HAuthorization:<redacted> <redacted>; ls",
        ),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn the_credential_after_bearer_or_basic_is_redacted() {
    let cases = [
        ("echo Bearer abc def", "echo Bearer <redacted> def"),
        ("printf 'basic dXNlcjpwYXNz'", "printf 'basic <redacted>'"),
        ("echo BASIC Abc", "echo BASIC <redacted>"),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn a_plain_lowercase_word_after_basic_is_kept() {
    assert_eq!(redact("echo basic usage"), "echo basic usage");
}

#[test]
fn url_userinfo_is_redacted() {
    let cases = [
        (
            "git clone https://user:tok@github.com/o/r.git",
            "git clone https://<redacted>@github.com/o/r.git",
        ),
        (
            "curl \"https://ci:abc@host/p?q=1\"",
            "curl \"https://<redacted>@host/p?q=1\"",
        ),
        (
            "tool --url=ftp://tok@h,ssh://u:p@k/x",
            "tool --url=ftp://<redacted>@h,ssh://<redacted>@k/x",
        ),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn urls_without_userinfo_are_kept() {
    let cases = [
        "git clone git@github.com:o/r.git",
        "curl https://host/a@b",
        "open https://host?to=a@b",
    ];
    for command in cases {
        assert_eq!(redact(command), command, "{command:?}");
    }
}

#[test]
fn secret_short_options_of_known_tools_are_redacted() {
    let cases = [
        ("mysql -uroot -phunter2 db", "mysql -uroot -p<redacted> db"),
        (
            "sudo /usr/bin/mysqldump -p'p w' db",
            "sudo /usr/bin/mysqldump -p<redacted> db",
        ),
        (
            "curl -u user:pass https://x",
            "curl -u <redacted> https://x",
        ),
        ("curl -su user:pass x", "curl -su <redacted> x"),
        ("curl -uuser:pass x", "curl -u<redacted> x"),
        ("curl --user user:pass x", "curl --user <redacted> x"),
        ("curl --proxy-user=u:p x", "curl --proxy-user=<redacted> x"),
        ("sshpass -p pw ssh h", "sshpass -p <redacted> ssh h"),
        ("redis-cli -a pw ping", "redis-cli -a <redacted> ping"),
        ("mongosh -u me -p pw", "mongosh -u me -p <redacted>"),
        ("bash -c 'mysql -pX'", "bash -c 'mysql -p<redacted>'"),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn short_options_of_other_tools_are_kept() {
    let cases = [
        "mkdir -p a/b",
        "docker run -p 8080:80 img",
        "mysql -p db",
        "curl x; ls -u",
        "ssh -p 22 host",
        "git add -p",
    ];
    for command in cases {
        assert_eq!(redact(command), command, "{command:?}");
    }
}

#[test]
fn a_quote_opened_in_the_middle_of_a_value_is_redacted_through_its_close() {
    assert_eq!(redact("TOKEN=abc\"def ghi\" run"), "TOKEN=<redacted> run");
}

// Review round 1 of #89: each leak the reviewer saw reach the log in shadow mode.

#[test]
fn a_registry_login_password_option_is_redacted() {
    let cases = [
        (
            "docker login -u me -p hunter2 ghcr.io",
            "docker login -u me -p <redacted> ghcr.io",
        ),
        (
            "podman login -phunter2 quay.io",
            "podman login -p<redacted> quay.io",
        ),
        (
            "helm registry login h -p pw",
            "helm registry login h -p <redacted>",
        ),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn credential_named_header_values_are_redacted() {
    let cases = [
        (
            "curl -H \"X-Api-Key: abc\" -H 'PRIVATE-TOKEN: t' u",
            "curl -H \"X-Api-Key: <redacted>\" -H 'PRIVATE-TOKEN: <redacted>' u",
        ),
        (
            "curl -H \"Cookie: s=1; t=2\" u",
            "curl -H \"Cookie: <redacted>; <redacted>\" u",
        ),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn a_credential_key_inside_an_option_value_is_redacted() {
    assert_eq!(
        redact(
            "kubectl create secret generic s --from-literal=password=hunter2 --from-literal=user=me"
        ),
        "kubectl create secret generic s --from-literal=password=<redacted> --from-literal=user=me"
    );
}

#[test]
fn a_credential_system_property_is_redacted() {
    assert_eq!(
        redact("mvn deploy -Dpassword=hunter2 -Ddb.password=pw -Dskip=true"),
        "mvn deploy -Dpassword=<redacted> -Ddb.password=<redacted> -Dskip=true"
    );
}

#[test]
fn single_dash_credential_options_are_redacted() {
    assert_eq!(
        redact("keytool -importcert -storepass changeit -keypass k2 -alias a"),
        "keytool -importcert -storepass <redacted> -keypass <redacted> -alias a"
    );
}

#[test]
fn an_openssl_pass_phrase_is_redacted() {
    let cases = [
        (
            "openssl pkcs12 -export -passout pass:hunter2 -in c.pem",
            "openssl pkcs12 -export -passout <redacted> -in c.pem",
        ),
        (
            "openssl enc -k pass:hunter2 -in f",
            "openssl enc -k pass:<redacted> -in f",
        ),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn a_github_secret_body_is_redacted_and_other_bodies_kept() {
    let cases = [
        (
            "gh secret set TOKEN --body hunter2",
            "gh secret set TOKEN --body <redacted>",
        ),
        ("gh secret set K -bhunter2", "gh secret set K -b<redacted>"),
        ("gh pr create --body text", "gh pr create --body text"),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn a_quoted_or_escaped_program_name_is_still_a_known_tool() {
    let cases = [
        ("\"mysql\" -phunter2", "\"mysql\" -p<redacted>"),
        ("\\mysql -phunter2", "\\mysql -p<redacted>"),
        ("mysql '-phunter2' db", "mysql -p<redacted> db"),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn url_userinfo_holding_a_hash_is_redacted() {
    assert_eq!(
        redact("curl https://user:p#ss@host/x"),
        "curl https://<redacted>@host/x"
    );
}

#[test]
fn an_ldap_bind_password_is_redacted() {
    assert_eq!(
        redact("ldapsearch -x -D cn=admin -w hunter2 -b dc=x"),
        "ldapsearch -x -D cn=admin -w <redacted> -b dc=x"
    );
}

#[test]
fn a_sql_server_password_is_redacted() {
    assert_eq!(
        redact("sqlcmd -S db -U sa -P hunter2 -Q q"),
        "sqlcmd -S db -U sa -P <redacted> -Q q"
    );
}

#[test]
fn an_npm_auth_token_is_redacted() {
    assert_eq!(
        redact("npm config set //registry.npmjs.org/:_authToken=abc"),
        "npm config set //registry.npmjs.org/:_authToken=<redacted>"
    );
}

#[test]
fn a_quoted_authorization_key_has_its_value_redacted() {
    assert_eq!(
        redact("curl -d '{\"Authorization\": \"Token abc\"}' u"),
        "curl -d '{\"Authorization\": <redacted>' u"
    );
}

#[test]
fn a_credential_in_a_json_body_is_redacted() {
    let cases = [
        (
            "curl -d '{\"password\":\"hunter2\"}' u",
            "curl -d '{\"password\":<redacted>' u",
        ),
        (
            "curl -d '{\"password\": \"hunter2\", \"user\": \"me\"}' u",
            "curl -d '{\"password\": <redacted> <redacted> <redacted>' u",
        ),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn an_smb_user_and_password_is_redacted() {
    assert_eq!(
        redact("smbclient //h/s -U me%hunter2"),
        "smbclient //h/s -U <redacted>"
    );
}

#[test]
fn an_obvious_token_is_redacted_in_a_command() {
    assert_eq!(
        redact("echo ghp_0123456789abcdefghij | gh auth login"),
        "echo <redacted> | gh auth login"
    );
}

#[test]
fn a_credential_key_with_its_value_in_the_word_hides_only_that_word() {
    assert_eq!(
        redact("git push origin auth-fix:main --force"),
        "git push origin auth-fix:<redacted> --force"
    );
}

#[test]
fn a_path_separator_is_not_a_key() {
    let command = "cargo test auth::tests::login";
    assert_eq!(redact(command), command);
}

// Review round 2 of #89.

#[test]
fn a_header_in_a_shell_body_does_not_hide_the_next_command() {
    let cases = [
        (
            "bash -c 'curl -H X-Api-Key: k; dropdb production'",
            "bash -c 'curl -H X-Api-Key: <redacted>; dropdb production'",
        ),
        (
            "x=$(echo token: a; kubectl delete ns prod)",
            "x=$(echo token: <redacted>; kubectl delete ns prod)",
        ),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn cookie_pairs_after_a_semicolon_in_a_quote_stay_hidden_but_a_command_does_not() {
    let cases = [
        (
            "curl -H 'Cookie: a=1; b=2; c=3' u",
            "curl -H 'Cookie: <redacted>; <redacted>; <redacted>' u",
        ),
        (
            "bash -c 'curl -H Cookie: a=1; b=2 rm -rf /tmp/x'",
            "bash -c 'curl -H Cookie: <redacted>; <redacted> rm -rf /tmp/x'",
        ),
        (
            "bash -c 'curl -H Cookie: a=1 | b=2 sh'",
            "bash -c 'curl -H Cookie: <redacted> | b=2 sh'",
        ),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
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
                    redacted.contains("dropdb production"),
                    "{command:?} became {redacted:?}"
                );
            }
        }
    }
}

#[test]
fn a_credential_config_key_has_its_next_word_redacted() {
    let cases = [
        (
            "aws configure set aws_secret_access_key X",
            "aws configure set aws_secret_access_key <redacted>",
        ),
        (
            "git config user.password X",
            "git config user.password <redacted>",
        ),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn twine_and_az_login_passwords_are_redacted() {
    let cases = [
        (
            "twine upload -u me -p pw dist/x",
            "twine upload -u me -p <redacted> dist/x",
        ),
        ("az login -u me -p pw", "az login -u me -p <redacted>"),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn a_vault_token_is_redacted() {
    let cases = [
        ("vault login s.abcdefgh", "vault login <redacted>"),
        ("echo hvs.CAESIabcdef", "echo <redacted>"),
    ];
    for (command, expected) in cases {
        assert_eq!(redact(command), expected, "{command:?}");
    }
}

#[test]
fn a_credential_option_read_from_stdin_or_a_file_takes_no_value() {
    let command = "docker login --password-stdin -u me ghcr.io";
    assert_eq!(redact(command), command);
}

#[test]
fn url_userinfo_holding_a_question_mark_is_redacted() {
    assert_eq!(
        redact("curl https://user:p?X@host/x"),
        "curl https://<redacted>@host/x"
    );
}

// Final review of #89.

#[test]
fn a_cookie_with_no_blank_after_its_colon_hides_every_pair() {
    assert_eq!(
        redact("curl -H \"Cookie:a=x; sid=S\" u"),
        "curl -H \"Cookie:<redacted>; <redacted>\" u"
    );
}

#[test]
fn a_cookie_pair_with_a_quoted_value_is_hidden() {
    assert_eq!(
        redact("curl -H 'Cookie: a=x; sid=\"S\"' u"),
        "curl -H 'Cookie: <redacted>; <redacted>' u"
    );
}

#[test]
fn a_cookie_pair_ending_in_the_quote_that_closes_its_header_is_hidden() {
    assert_eq!(
        redact("bash -c \"curl -H 'Cookie: a=S; sid=S' u\""),
        "bash -c \"curl -H 'Cookie: <redacted>; <redacted> u\""
    );
}

#[test]
fn a_word_after_a_cookie_semicolon_holding_a_substitution_stays_readable() {
    assert_eq!(
        redact("bash -c 'curl -H Cookie: a=1; b=$(dropdb production)'"),
        "bash -c 'curl -H Cookie: <redacted>; b=$(dropdb production)'"
    );
}

#[test]
fn a_word_after_a_cookie_semicolon_that_is_no_pair_ends_the_hidden_pairs() {
    assert_eq!(
        redact("curl -H 'Cookie: a=1; Secure; b=2' u"),
        "curl -H 'Cookie: <redacted>; Secure; b=2' u"
    );
}

#[test]
fn a_query_holding_an_at_after_a_port_is_redacted_as_userinfo() {
    // Accepted over-redaction (wi-434adf): `host:8080?to=a` reads like `user:pass?X`.
    assert_eq!(
        redact("open https://host:8080?to=a@b"),
        "open https://<redacted>@b"
    );
}
