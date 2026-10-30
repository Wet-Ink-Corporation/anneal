use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::time::Duration;

use super::{Client, Request};
use crate::decide::transport::ApiToken;
use crate::verdict::commit_status::{PostError, State};

/// What the fake read of the one request it served: the request line and headers, then the
/// body.
#[derive(Debug, Default)]
struct Served {
    head: String,
    body: String,
}

impl Served {
    fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name).then(|| value.trim())
        })
    }
}

/// Reads one request from `listener` and answers `response`; `None` answers nothing and waits
/// for the client to hang up.
fn serve_one(listener: &TcpListener, response: Option<&str>) -> Served {
    let (stream, _) = listener.accept().expect("a connection");
    let mut reader = BufReader::new(stream.try_clone().expect("a second handle"));
    let mut served = Served::default();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).expect("a header line") == 0 || line == "\r\n" {
            break;
        }
        served.head.push_str(&line);
    }
    let length: usize = served
        .header("content-length")
        .map_or(0, |value| value.parse().expect("a numeric content-length"));
    let mut body = vec![0; length];
    reader.read_exact(&mut body).expect("the declared body");
    served.body = String::from_utf8(body).expect("a UTF-8 body");
    let mut stream = stream;
    match response {
        Some(response) => stream.write_all(response.as_bytes()).expect("the answer"),
        // Blocks until the client gives up and closes the connection.
        None => drop(reader.read(&mut [0; 1]).expect("the client hangs up")),
    }
    served
}

/// Posts a status through a client given `timeout`, to a fake on a loopback port that answers
/// `response`; returns the post's result and what the fake read.
fn post_to_fake(response: Option<&str>, timeout: Duration) -> (Result<(), PostError>, Served) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let addr = listener.local_addr().expect("its address");
    let token = ApiToken::new("t0k".to_owned()).expect("a valid token");
    std::thread::scope(|scope| {
        let server = std::thread::Builder::new()
            .name("fake-github".to_owned())
            .spawn_scoped(scope, || serve_one(&listener, response))
            .expect("a server thread");
        let request = Request {
            url: format!("http://{addr}/repos/owner/repo/statuses/abc123"),
            token: &token,
            state: State::Pending,
            description: "tests-verified by local: the merge gate wants review",
        };
        let result = Client::new(timeout).post(&request);
        (result, server.join().expect("the fake GitHub"))
    })
}

const CREATED: &str = "HTTP/1.1 201 Created\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}";

#[test]
fn a_status_is_posted_with_the_token_context_state_and_description() {
    let (result, served) = post_to_fake(Some(CREATED), Duration::from_secs(10));

    assert!(result.is_ok(), "{result:?}");
    assert!(
        served
            .head
            .starts_with("POST /repos/owner/repo/statuses/abc123 HTTP/1.1\r\n"),
        "{}",
        served.head
    );
    assert_eq!(served.header("authorization"), Some("Bearer t0k"));
    assert_eq!(served.header("accept"), Some("application/vnd.github+json"));
    assert_eq!(served.header("x-github-api-version"), Some("2022-11-28"));
    let body: serde_json::Value = serde_json::from_str(&served.body).expect("a JSON body");
    assert_eq!(
        body,
        serde_json::json!({
            "state": "pending",
            "context": "anneal/verdict",
            "description": "tests-verified by local: the merge gate wants review",
        })
    );
}

#[test]
fn a_refused_status_names_the_code_and_github_s_message() {
    let body = r#"{"message":"Validation Failed","documentation_url":"https://docs.github.com"}"#;
    let response = format!(
        "HTTP/1.1 422 Unprocessable Entity\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );

    let (result, _) = post_to_fake(Some(&response), Duration::from_secs(10));

    let message = result.err().map(|error| error.to_string());
    assert_eq!(
        message.as_deref(),
        Some("GitHub answered HTTP 422: Validation Failed")
    );
}

#[test]
fn a_message_with_control_characters_or_too_long_is_cleaned_and_cut() {
    let long = "x".repeat(500);
    let body = format!(r#"{{"message":"bad\u001b[31m\n{long}"}}"#);
    let response = format!(
        "HTTP/1.1 403 Forbidden\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );

    let (result, _) = post_to_fake(Some(&response), Duration::from_secs(10));

    let message = result
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(
        message.starts_with("GitHub answered HTTP 403: bad[31m"),
        "{message}"
    );
    assert!(!message.chars().any(char::is_control), "{message:?}");
    assert!(message.chars().count() < 200, "{message}");
}

#[test]
fn an_error_body_over_the_cap_is_not_quoted() {
    let body = format!(r#"{{"message":"{}"}}"#, "y".repeat(20_000));
    let response = format!(
        "HTTP/1.1 500 Internal Server Error\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );

    let (result, _) = post_to_fake(Some(&response), Duration::from_secs(10));

    let message = result.err().map(|error| error.to_string());
    assert_eq!(message.as_deref(), Some("GitHub answered HTTP 500"));
}

#[test]
fn a_redirect_is_not_followed() {
    let response = "HTTP/1.1 302 Found\r\nlocation: http://127.0.0.1:1/elsewhere\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";

    let (result, _) = post_to_fake(Some(response), Duration::from_secs(10));

    assert!(
        matches!(result, Err(PostError::Status { code: 302, .. })),
        "{result:?}"
    );
}

#[test]
fn a_server_that_never_answers_times_out() {
    let (result, served) = post_to_fake(None, Duration::from_millis(300));

    assert!(
        matches!(result, Err(PostError::TimedOut { .. })),
        "{result:?}"
    );
    assert!(served.head.starts_with("POST "), "the request was sent");
}

#[test]
fn a_refused_connection_is_unreachable() {
    let token = ApiToken::new("t0k".to_owned()).expect("a valid token");
    let request = Request {
        url: "http://127.0.0.1:0/repos/owner/repo/statuses/abc123".to_owned(),
        token: &token,
        state: State::Success,
        description: "d",
    };

    let result = Client::new(Duration::from_secs(10)).post(&request);

    assert!(
        matches!(result, Err(PostError::Unreachable { .. })),
        "{result:?}"
    );
    let message = result
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(
        !message.contains("t0k") && !message.contains("127.0.0.1"),
        "{message}"
    );
}
