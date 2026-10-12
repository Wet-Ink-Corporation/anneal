//! `anneal decide` over the real HTTP transport, against a fake Workers AI on a loopback port
//! (`ANNEAL_CLEF_URL`): the binary is wired to the adapter, sends the token only in the
//! `Authorization` header, takes no proxy from the environment, and refuses an override that is
//! not on the loopback host.

use std::error::Error;
use std::net::TcpListener;
use std::process::Output;

use serde_json::{Value, json};

use crate::support::{anneal_in, with_fake_clef};

type TestResult = Result<(), Box<dyn Error>>;

const SECRET: &str = "s3cr3t-T0KEN-value";
const ENDPOINT_VAR: &str = "ANNEAL_CLEF_URL";

const D1: [&str; 7] = [
    "decide",
    "--point",
    "D1",
    "--question",
    "noul",
    "--state",
    r#"{"tool":"Bash","input":{"command":"rm -rf /"},"phase":"build"}"#,
];

/// Workers AI's reply with D1's `yes` at 0.95, which its bands make `deny`.
fn answer() -> String {
    let inner = r#"{"probabilities":{"yes":0.95,"no":0.05}}"#;
    json!({"result": {"response": inner}, "success": true}).to_string()
}

fn envelope(output: &Output) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&output.stdout)?)
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary and opens sockets")]
fn an_answer_from_clef_exits_0_with_the_action_and_one_row() -> TestResult {
    let home = tempfile::tempdir()?;

    let (output, served) = with_fake_clef(&answer(), |base| {
        anneal_in(home.path())
            .args(D1)
            .env("ANNEAL_HOME", home.path())
            .env("CLAUDE_PLUGIN_OPTION_CF_ACCOUNT_ID", "acct0123")
            .env("CLAUDE_PLUGIN_OPTION_CF_API_TOKEN", SECRET)
            .env(ENDPOINT_VAR, base)
            .output()
    });
    let output = output?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(envelope(&output)?["data"]["action"], json!("deny"));
    assert!(
        served
            .head
            .starts_with("POST /accounts/acct0123/ai/run/@cf/wet-ink/clef-flash HTTP/1.1"),
        "{}",
        served.head
    );
    assert_eq!(
        served.header("authorization"),
        Some(format!("Bearer {SECRET}").as_str())
    );
    let sent: Value = serde_json::from_slice(&served.body)?;
    assert_eq!(sent["messages"][0]["role"], json!("system"));
    let log = std::fs::read_to_string(home.path().join("decisions").join("decisions.jsonl"))?;
    assert_eq!(log.lines().count(), 1, "{log}");
    for shown in [
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        log,
    ] {
        assert!(!shown.contains(SECRET), "{shown}");
    }
    Ok(())
}

/// ureq reads `ALL_PROXY`, `HTTPS_PROXY` and `HTTP_PROXY` by default; the adapter turns that
/// off, so a proxy in the environment never sees the request or the token.
#[test]
#[cfg_attr(miri, ignore = "runs the binary and opens sockets")]
fn a_proxy_in_the_environment_is_not_used() -> TestResult {
    let home = tempfile::tempdir()?;
    let proxy = TcpListener::bind("127.0.0.1:0")?;
    proxy.set_nonblocking(true)?;
    let proxy_url = format!("http://{}", proxy.local_addr()?);

    let (output, served) = with_fake_clef(&answer(), |base| {
        let mut command = anneal_in(home.path());
        command
            .args(D1)
            .env("ANNEAL_HOME", home.path())
            .env("CLAUDE_PLUGIN_OPTION_CF_ACCOUNT_ID", "acct0123")
            .env("CLAUDE_PLUGIN_OPTION_CF_API_TOKEN", SECRET)
            .env(ENDPOINT_VAR, base)
            .env_remove("NO_PROXY")
            .env_remove("no_proxy");
        for var in [
            "ALL_PROXY",
            "HTTPS_PROXY",
            "HTTP_PROXY",
            "all_proxy",
            "https_proxy",
            "http_proxy",
        ] {
            command.env(var, &proxy_url);
        }
        command.output()
    });
    let output = output?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        served.head.starts_with("POST /accounts/"),
        "{}",
        served.head
    );
    assert_eq!(
        proxy.accept().map(|_| ()).map_err(|error| error.kind()),
        Err(std::io::ErrorKind::WouldBlock),
        "the proxy saw no connection"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn an_endpoint_off_the_loopback_host_is_misconfigured_and_sends_nothing() -> TestResult {
    let home = tempfile::tempdir()?;

    let output = anneal_in(home.path())
        .args(D1)
        .env("ANNEAL_HOME", home.path())
        .env("CLAUDE_PLUGIN_OPTION_CF_ACCOUNT_ID", "acct0123")
        .env("CLAUDE_PLUGIN_OPTION_CF_API_TOKEN", SECRET)
        .env(ENDPOINT_VAR, "https://example.com/accounts")
        .output()?;

    assert_eq!(output.status.code(), Some(5), "{output:?}");
    let envelope = envelope(&output)?;
    assert_eq!(envelope["data"]["error"], json!("misconfigured"));
    assert_eq!(envelope["data"]["fallback"], json!("ask"));
    assert!(
        envelope["data"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.starts_with("ANNEAL_CLEF_URL must be http://")),
        "{envelope}"
    );
    assert!(
        !home.path().join("decisions").exists(),
        "nothing asked or logged"
    );
    Ok(())
}
