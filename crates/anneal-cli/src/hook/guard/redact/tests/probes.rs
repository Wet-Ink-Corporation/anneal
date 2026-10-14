//! The invariant on every known leak: no secret of a probe reaches the skeleton. The probes are
//! the secrets of this module's other tests, the 50 probes of the parked round-4 scanner
//! (`wip-redaction-89`, dab072d) and its review's findings (secrets in `$( )` bodies, commands
//! hidden after wrappers, ANSI-C decoding). Where a probe's secret was one letter, it is
//! spelled out (`k5ecret`) so that its absence means something.

use super::super::redact;

/// Asserts that no secret of any probe is in its skeleton.
fn assert_hidden(probes: &[(&str, &[&str])]) {
    for &(command, secrets) in probes {
        let redacted = redact(command);
        for secret in secrets {
            assert!(
                !redacted.contains(secret),
                "{secret:?} of {command:?} in {redacted:?}"
            );
        }
    }
}

/// A command and the secrets it holds.
type Probe = (&'static str, &'static [&'static str]);

/// The secrets of the scanner's tests, which `super` now pins to the skeleton.
const EARLIER: &[Probe] = &[
    ("FOO=bar5ecret cargo test", &["bar5ecret"]),
    (
        "A=1 B='two words' C=\"x $(cat secret) y\" run",
        &["two words", "secret"],
    ),
    ("TOKEN=$(pass show ci) && X=y deploy", &["show"]),
    ("export API_KEY=abc", &["abc"]),
    ("export \"db_\"'token'=hunter2 run", &["hunter2", "token"]),
    (
        "# let's deploy\nAPI_KEY=sk-live-123 ./deploy.sh",
        &["sk-live-123", "let's"],
    ),
    ("echo \"$'\" ; TOKEN=abc deploy", &["abc"]),
    ("cat <<EOF\nit's\nEOF\nTOKEN=x run", &["it's"]),
    ("cat <<-'END' > f\n\tsecret: x\n\tEND\nls", &["secret"]),
    ("cat <<<\"TOKEN=hunter2\"", &["hunter2"]),
    ("echo 'abc TOKEN=x", &["abc"]),
    ("ls $(echo TOKEN=hunter2", &["hunter2"]),
    ("curl --token=abc https://x", &["abc", "https"]),
    ("login --password hunter2 --user me", &["hunter2"]),
    ("tool --api-key='k k' go", &["k k"]),
    ("vault --client-secret s5ecret", &["s5ecret"]),
    (
        "SECRET=hhhhhhhhhhhhhhhhhhhh rm -r build",
        &["hhhh", "build"],
    ),
    ("K='abc def", &["abc"]),
    (
        "curl -H \"Authorization: Bearer abc\" https://x",
        &["abc", "Bearer"],
    ),
    ("curl -HAuthorization:Basic\\ abc u; ls", &["abc", "Basic"]),
    ("printf 'basic dXNlcjpwYXNz'", &["dXNlcjpwYXNz"]),
    (
        "git clone https://user:tok@github.com/o/r.git",
        &["user:tok", "github"],
    ),
    ("curl \"https://ci:abc@host/p?q=1\"", &["ci:abc"]),
    ("tool --url=ftp://tok@h,ssh://u:p@k/x", &["tok@", "u:p@"]),
    ("mysql -uroot -phunter2 db", &["hunter2", "root"]),
    ("sudo /usr/bin/mysqldump -p'p w' db", &["p w"]),
    ("curl -u user:pass https://x", &["user:pass"]),
    ("curl -su user:pass x", &["user:pass"]),
    ("curl -uuser:pass x", &["user:pass"]),
    ("curl --user user:pass x", &["user:pass"]),
    ("curl --proxy-user=u:p x", &["u:p"]),
    ("sshpass -p pw5ecret ssh h", &["pw5ecret"]),
    ("redis-cli -a pw5ecret ping", &["pw5ecret"]),
    ("mongosh -u me -p pw5ecret", &["pw5ecret"]),
    ("bash -c 'mysql -pX5ecret'", &["X5ecret"]),
    ("TOKEN=abc\"def ghi\" run", &["abc", "def ghi"]),
    ("docker login -u me -p hunter2 ghcr.io", &["hunter2"]),
    ("podman login -phunter2 quay.io", &["hunter2"]),
    ("helm registry login h -p pw5ecret", &["pw5ecret"]),
    (
        "curl -H \"X-Api-Key: abc\" -H 'PRIVATE-TOKEN: t5ecret' u",
        &["abc", "t5ecret"],
    ),
    ("curl -H \"Cookie: s=1; t=2\" u", &["s=1", "t=2"]),
    (
        "kubectl create secret generic s --from-literal=password=hunter2",
        &["hunter2"],
    ),
    (
        "mvn deploy -Dpassword=hunter2 -Ddb.password=pw5ecret",
        &["hunter2", "pw5ecret"],
    ),
    (
        "keytool -importcert -storepass changeit -keypass k2k2",
        &["changeit", "k2k2"],
    ),
    (
        "openssl pkcs12 -export -passout pass:hunter2 -in c.pem",
        &["hunter2"],
    ),
    ("openssl enc -k pass:hunter2 -in f", &["hunter2"]),
    ("gh secret set TOKEN --body hunter2", &["hunter2"]),
    ("gh secret set K -bhunter2", &["hunter2"]),
    ("\"mysql\" -phunter2", &["hunter2"]),
    ("\\mysql -phunter2", &["hunter2"]),
    ("mysql '-phunter2' db", &["hunter2"]),
    ("curl https://user:p#ss@host/x", &["p#ss"]),
    ("ldapsearch -x -D cn=admin -w hunter2 -b dc=x", &["hunter2"]),
    ("sqlcmd -S db -U sa -P hunter2 -Q q", &["hunter2"]),
    (
        "npm config set //registry.npmjs.org/:_authToken=abc",
        &["abc", "authToken"],
    ),
    (
        "curl -d '{\"Authorization\": \"Token abc\"}' u",
        &["Token abc"],
    ),
    ("curl -d '{\"password\":\"hunter2\"}' u", &["hunter2"]),
    ("smbclient //h/s -U me%hunter2", &["hunter2"]),
    (
        "echo ghp_0123456789abcdefghij | gh auth login",
        &["ghp_0123456789abcdefghij"],
    ),
    ("vault login s.abcdefgh", &["s.abcdefgh"]),
    ("echo hvs.CAESIabcdef", &["hvs.CAESIabcdef"]),
    ("curl https://user:p?X@host/x", &["p?X"]),
    ("curl -H \"Cookie:a=x; sid=S5ecret\" u", &["S5ecret"]),
    ("curl -H 'Cookie: a=x; sid=\"S5ecret\"' u", &["S5ecret"]),
    ("docker run -v db-password:/p alpine", &["db-password"]),
    ("git push origin fix-auth:main --force", &["fix-auth"]),
    (
        "aws configure set aws_secret_access_key X5ecret",
        &["X5ecret"],
    ),
    ("git config user.password X5ecret", &["X5ecret"]),
    ("twine upload -u me -p pw5ecret dist/x", &["pw5ecret"]),
    ("az login -u me -p pw5ecret", &["pw5ecret"]),
];

#[test]
fn no_secret_of_the_earlier_redaction_tests_reaches_the_skeleton() {
    assert_hidden(EARLIER);
}

/// The 50 probes of the round-4 scanner's binary run, in its order.
const ROUND_4: &[Probe] = &[
    (
        "bash -c 'curl -H \"Authorization: Bearer $TOKEN\" https://x; dropdb production'",
        &["Bearer", "$TOKEN", "production"],
    ),
    (
        "sh -c 'curl -H \"X-Api-Key: $K\" https://x; dropdb production'",
        &["X-Api-Key", "$K"],
    ),
    (
        "eval 'curl -H \"Cookie: sid=$S\" u; dropdb production'",
        &["sid=", "Cookie"],
    ),
    (
        "ssh host 'curl -H \"Authorization: Bearer $T\" u; dropdb production'",
        &["Bearer", "host"],
    ),
    (
        "find . -exec sh -c 'curl -H \"Authorization: Bearer $T\" u; rm -r \"$1\"' _ {} \\;",
        &["Bearer"],
    ),
    (
        "x=$(curl -H \"Authorization: Bearer $T\" https://x; dropdb production)",
        &["Bearer", "production"],
    ),
    (
        "echo \"$(curl -sH \"Authorization: token $GH\" https://x)\" && dropdb production",
        &["token", "$GH"],
    ),
    (
        "bash -c \"curl -H 'X-Api-Key: '\\\\\n s5ecret u\"",
        &["s5ecret"],
    ),
    (
        "bash -c \"curl -H 'X-Auth-Token:Bearer'\\\\\n xyz u\"",
        &["xyz", "Bearer"],
    ),
    (
        "curl -H \"a=b; Authorization: Bearer\" tok5ecret u",
        &["tok5ecret", "Bearer"],
    ),
    ("curl -H \"x=1 X-Api-Key:\" s5ecret u", &["s5ecret"]),
    (
        "sudo -p 'Password: ' dropdb production",
        &["Password", "production"],
    ),
    (
        "curl -H $'X-Auth-Token:\\x42earer' tok5ecret u",
        &["tok5ecret", "earer"],
    ),
    ("echo gh\"p_abcdefghijklmnop\" u", &["abcdefghijklmnop"]),
    ("curl -H X-Auth-Token:Bearer\\\n  xyz u", &["xyz", "Bearer"]),
    ("x=\"Bearer\" dropdb production", &["Bearer"]),
    (
        "curl -H \"X-Api-Key:\"'k;dropdb' u",
        &["X-Api-Key", "k;dropdb"],
    ),
    (
        "curl -H \"X-Api-Key:\"'s5ecret value' u",
        &["s5ecret value"],
    ),
    ("curl -H 'X-Api-Key:k'\"more5ecret\" u", &["more5ecret"]),
    ("curl -H \"X-Auth-Token:Bearer \"x'y' u", &["Bearer"]),
    (
        "bash -c 'curl -H '\\''X-Api-Key: '\\''$K; dropdb production'",
        &["X-Api-Key", "$K"],
    ),
    (
        "bash -c 'curl -H \"X-Api-Key:\"'\\''k5ecret'\\''; dropdb production'",
        &["X-Api-Key", "k5ecret"],
    ),
    (
        "sh -c \"curl -H 'Authorization: Bearer '\\\"$T\\\"'; dropdb production'\"",
        &["Bearer"],
    ),
    (
        "TOKEN=abc$(cat k; dropdb production)def5ecret deploy",
        &["abc", "def5ecret"],
    ),
    ("curl -u me:pw$(get; dropdb production) u", &["me:pw"]),
    (
        "echo `curl -H \"Authorization: Bearer $T\" u; dropdb production`",
        &["Bearer"],
    ),
    (
        "x=(a $(curl -H \"X-Api-Key: k5ecret\" u; dropdb production))",
        &["X-Api-Key", "k5ecret"],
    ),
    (
        "env -u HOME x='Bearer' dropdb production",
        &["Bearer", "HOME"],
    ),
    ("export x='Bearer' dropdb", &["Bearer"]),
    ("sudo x='Bearer' dropdb production", &["Bearer"]),
    (
        "eval \"curl -H 'X-Api-Key: '\\\\\n k5ecret; dropdb production\"",
        &["X-Api-Key", "k5ecret"],
    ),
    ("echo gh\"p_abcdefghijklmnop;x\"", &["abcdefghijklmnop"]),
    (
        "bash -c $'curl -H \"X-Api-Key: k5ecret\"\\ndropdb production'",
        &["X-Api-Key", "k5ecret"],
    ),
    (
        "bash -c \"curl -H 'Authorization: Bearer x5ecret'; dropdb production\"",
        &["Bearer", "x5ecret"],
    ),
    (
        "curl -H \"Authorization: Bearer\" -- \"$(dropdb production)\"",
        &["Bearer"],
    ),
    (
        "printf '%s' \"k=v Bearer\" tok5ecret",
        &["Bearer", "tok5ecret"],
    ),
    ("\"x\"=Bearer dropdb production", &["Bearer"]),
    ("time x='Bearer' dropdb production", &["Bearer"]),
    (
        "x='Bearer' y=\"Basic\" dropdb production",
        &["Bearer", "Basic"],
    ),
    (
        "curl -H 'X-Api-Key:'$'\\x73ecret' u",
        &["secret", "x73ecret"],
    ),
    (
        "curl -H \"Authorization: Basic \"dXNlcjpwYXNz u",
        &["dXNlcjpwYXNz"],
    ),
    (
        "watch -n1 'curl -H \"X-Api-Key: k5ecret\" u; dropdb production'",
        &["X-Api-Key", "k5ecret"],
    ),
    (
        "su -c 'curl -H \"Authorization: Bearer x5ecret\" u; dropdb production' root",
        &["Bearer", "x5ecret", "root"],
    ),
    (
        "xargs -I{} sh -c 'curl -H \"Cookie: a={}\" u; dropdb production'",
        &["Cookie"],
    ),
    (
        "bash -c \"$(printf 'curl -H %s u; dropdb production' 'X-Api-Key:k5ecret')\"",
        &["X-Api-Key", "k5ecret"],
    ),
    (
        "eval \"X-Api-Key: \"'k5ecret; dropdb production'",
        &["X-Api-Key", "k5ecret"],
    ),
    ("TOKEN=$'abc\\ndef' deploy", &["abc", "def"]),
    ("curl -u $'me:p\\x61ss' u", &["me:pass", "me:p"]),
    ("echo \"https://me:p\"'w@host/;x' u", &["me:pw"]),
    (
        "bash -c \"echo \"gh\"p_abcdefghijklmnop; dropdb production\"",
        &["abcdefghijklmnop"],
    ),
];

#[test]
fn no_secret_of_the_round_4_probes_reaches_the_skeleton() {
    assert_eq!(ROUND_4.len(), 50, "every probe of the round-4 run");

    assert_hidden(ROUND_4);
}

#[test]
fn no_secret_of_the_round_4_review_findings_reaches_the_skeleton() {
    assert_hidden(&[
        // Secrets in `$( )` bodies in secret positions.
        (
            "curl -H \"Authorization: Bearer $(echo s3cr3tval)\" u",
            &["s3cr3tval", "Bearer"],
        ),
        ("mysql -p$(printf s3cr3tval) db", &["s3cr3tval"]),
        ("TOKEN=$(printf s3cr3tval) deploy", &["s3cr3tval"]),
        ("curl -u `echo me:s3cr3tval` u", &["s3cr3tval"]),
        // ANSI-C escapes, decoded as bash decodes them.
        (
            "curl -H $'Authorization: \\x42earer s3cr3tval' u",
            &["s3cr3tval", "Bearer"],
        ),
        ("echo $'\\x73\\x33cr3tval'", &["s3cr3tval"]),
        (
            "bash -c $'echo s3cr3tval\\x3b dropdb production'",
            &["s3cr3tval"],
        ),
    ]);
}

#[test]
fn a_command_after_any_wrapper_stays_visible_and_its_values_do_not() {
    let wrappers = [
        "nohup",
        "timeout 5",
        "nice -n 5",
        "exec",
        "command",
        "nohup timeout 5 nice exec command",
    ];
    for wrapper in wrappers {
        let command = format!("{wrapper} env x='Bearer' dropdb production");

        let redacted = redact(&command);

        assert!(
            redacted.contains("dropdb")
                && !redacted.contains("Bearer")
                && !redacted.contains("production"),
            "{command:?} became {redacted:?}"
        );
    }
}
