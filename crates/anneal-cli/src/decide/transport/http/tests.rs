//! The adapter against a fake Workers AI on a loopback port: the narrowest real seam for an HTTP
//! client (TST-02). Each fake serves one connection on a scoped thread that is always joined.

use std::ffi::OsString;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use super::{READ_CAP, UreqTransport};
use crate::decide::client::{self, Ask, Outcome, Setup};
use crate::decide::clock::SystemClock;
use crate::decide::config::{ACCOUNT_VAR, ClefConfig, ENDPOINT_VAR, TOKEN_VAR};
use crate::decide::log::DecisionLog;
use crate::decide::point::{Point, Question};
use crate::decide::state::CompactState;
use crate::decide::thresholds::Thresholds;
use crate::decide::transport::{
    ApiToken, ClefRequest, MAX_RESPONSE_BYTES, Transport, TransportError,
};

const SECRET: &str = "s3cr3t-T0KEN-value";
const TIMEOUT: Duration = Duration::from_secs(5);

/// What the fake server read of the one request it served.
#[derive(Debug, Default)]
struct Received {
    head: String,
    body: Vec<u8>,
}

impl Received {
    /// The value of header `name` (case-insensitive), if sent.
    fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name).then(|| value.trim())
        })
    }
}

/// Accepts one connection, reads its request and writes `response`, closing the connection. A
/// dummy connection the test makes to unblock `accept` reads as an empty request.
fn serve_one(listener: &TcpListener, response: &[u8]) -> Received {
    serve_and_hold(listener, response).0
}

/// As [`serve_one`], handing back the connection open.
fn serve_and_hold(listener: &TcpListener, response: &[u8]) -> (Received, TcpStream) {
    let (stream, _) = listener.accept().expect("a connection");
    stream
        .set_read_timeout(Some(TIMEOUT))
        .expect("a read timeout");
    let mut reader = BufReader::new(stream.try_clone().expect("a second handle"));
    let mut received = Received::default();
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).expect("a header line");
        if read == 0 || line == "\r\n" {
            break;
        }
        received.head.push_str(&line);
    }
    let length = received
        .header("content-length")
        .map_or(0, |value| value.parse().expect("a numeric content-length"));
    received.body = vec![0; length];
    reader
        .read_exact(&mut received.body)
        .expect("the declared body");
    let mut stream = stream;
    // The client may close early (an oversized body), so a failed write is not the test's
    // concern; what the client returned is.
    let wrote: io::Result<()> = stream.write_all(response);
    drop(wrote);
    (received, stream)
}

/// A request to `url` with the test token and `timeout`.
fn request<'a>(url: &str, token: &'a ApiToken, timeout: Duration) -> ClefRequest<'a> {
    ClefRequest {
        url: url.to_owned(),
        model: "@cf/test/model",
        body: br#"{"messages":[]}"#.to_vec(),
        token,
        timeout,
    }
}

fn token() -> ApiToken {
    ApiToken::new(SECRET.to_owned()).expect("a printable token")
}

fn url(addr: SocketAddr) -> String {
    format!("http://{addr}/client/v4/accounts/acct0123/ai/run/m")
}

/// Runs `client` against a fake that answers one connection with `response`, and returns what
/// `client` returned and what the fake read.
fn serving<T>(response: &[u8], client: impl FnOnce(SocketAddr) -> T) -> (T, Received) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let addr = listener.local_addr().expect("its address");
    thread::scope(|scope| {
        let server = thread::Builder::new()
            .name("fake-workers-ai".to_owned())
            .spawn_scoped(scope, || serve_one(&listener, response))
            .expect("a server thread");
        let result = client(addr);
        // Unblocks `accept` if the client never connected; otherwise it waits in the backlog.
        TcpStream::connect(addr).expect("a dummy connection");
        (result, server.join().expect("the fake server"))
    })
}

/// Sends one request to a fake that answers `response`, and returns the adapter's result and
/// what the fake read.
fn exchange(response: &[u8]) -> (Result<Vec<u8>, TransportError>, Received) {
    let token = token();
    serving(response, |addr| {
        UreqTransport::new().send(&request(&url(addr), &token, TIMEOUT))
    })
}

fn http_response(status: &str, extra: &str, body: &[u8]) -> Vec<u8> {
    let mut response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n{extra}connection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(body);
    response
}

/// The whole path: `client::ask` builds the request for the endpoint override, this adapter
/// sends it, and the answer is parsed and logged.
#[test]
#[cfg_attr(miri, ignore = "opens loopback sockets")]
fn a_200_answer_is_decided_end_to_end_through_the_client() {
    let home = tempfile::tempdir().expect("an anneal home");
    let thresholds = Thresholds::built_in().expect("the shipped thresholds");
    let log = DecisionLog::in_home(home.path());
    let state =
        CompactState::parse(r#"{"tool":"Bash","input":{"command":"rm -rf /"}}"#).expect("a state");
    let answer = r#"{"probabilities":{"yes":0.95,"no":0.05}}"#;
    let body = json!({"result": {"response": answer}, "success": true}).to_string();

    let (outcome, received) = serving(&http_response("200 OK", "", body.as_bytes()), |addr| {
        let base = format!("http://{addr}/accounts");
        let vars = [
            (ACCOUNT_VAR, "acct0123"),
            (TOKEN_VAR, SECRET),
            (ENDPOINT_VAR, base.as_str()),
        ];
        let config = ClefConfig::from_env(|name| {
            vars.iter()
                .find(|(var, _)| *var == name)
                .map(|(_, value)| OsString::from(value))
        })
        .expect("a valid configuration");
        let setup = Setup {
            thresholds: &thresholds,
            config: Ok(&config),
            log: &log,
        };
        let question = Ask {
            point: Point::D1,
            question: Question::Noul,
            options: &[],
            state: &state,
        };
        match client::ask(
            &question,
            &setup,
            &UreqTransport::new(),
            &SystemClock::new(),
        ) {
            Ok(Outcome::Decided(decision)) => decision.action.to_owned(),
            other => format!("not decided: {other:?}"),
        }
    });

    assert_eq!(outcome, "deny");
    assert!(
        received
            .head
            .starts_with("POST /accounts/acct0123/ai/run/@cf/wet-ink/clef-flash HTTP/1.1"),
        "{}",
        received.head
    );
    let rows = fs::read_to_string(home.path().join("decisions").join("decisions.jsonl"))
        .expect("the decision log");
    assert_eq!(rows.lines().count(), 1, "{rows}");
    assert!(!rows.contains(SECRET), "{rows}");
}

#[test]
#[cfg_attr(miri, ignore = "opens loopback sockets")]
fn a_2xx_body_is_returned_as_sent() {
    let body = br#"{"result":{"response":"{}"},"success":true}"#;

    let (result, _) = exchange(&http_response("200 OK", "", body));

    assert_eq!(result, Ok(body.to_vec()));
}

#[test]
#[cfg_attr(miri, ignore = "opens loopback sockets")]
fn the_request_carries_the_bearer_token_json_content_type_and_the_body() {
    let (_, received) = exchange(&http_response("200 OK", "", b"{}"));

    assert!(
        received
            .head
            .starts_with("POST /client/v4/accounts/acct0123/ai/run/m HTTP/1.1"),
        "{}",
        received.head
    );
    assert_eq!(
        received.header("authorization"),
        Some(format!("Bearer {SECRET}").as_str())
    );
    assert_eq!(received.header("content-type"), Some("application/json"));
    assert_eq!(received.body, br#"{"messages":[]}"#.to_vec());
}

#[test]
#[cfg_attr(miri, ignore = "opens loopback sockets")]
fn a_body_of_exactly_the_cap_is_accepted() {
    let body = vec![b' '; MAX_RESPONSE_BYTES];

    let (result, _) = exchange(&http_response("200 OK", "", &body));

    assert_eq!(result.map(|bytes| bytes.len()), Ok(MAX_RESPONSE_BYTES));
}

#[test]
#[cfg_attr(miri, ignore = "opens loopback sockets")]
fn a_body_over_the_cap_is_a_bad_response() {
    let body = vec![b' '; MAX_RESPONSE_BYTES + 1];

    let (result, _) = exchange(&http_response("200 OK", "", &body));

    assert_eq!(
        result,
        Err(TransportError::BadResponse {
            reason: "response body over 65536 bytes".to_owned()
        })
    );
}

#[test]
#[cfg_attr(miri, ignore = "opens loopback sockets")]
fn a_chunked_body_over_the_cap_is_a_bad_response_too() {
    let chunk = vec![b' '; MAX_RESPONSE_BYTES];
    let mut response =
        b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n".to_vec();
    for _ in 0..2 {
        response.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
        response.extend_from_slice(&chunk);
        response.extend_from_slice(b"\r\n");
    }
    response.extend_from_slice(b"0\r\n\r\n");

    let (result, _) = exchange(&response);

    assert!(
        matches!(result, Err(TransportError::BadResponse { .. })),
        "{result:?}"
    );
}

#[test]
fn the_read_cap_is_one_byte_past_the_largest_body() {
    assert_eq!(usize::try_from(READ_CAP), Ok(MAX_RESPONSE_BYTES + 1));
}

#[test]
#[cfg_attr(miri, ignore = "opens loopback sockets")]
fn a_non_2xx_status_is_a_bad_response_naming_the_code() {
    for (status, code) in [
        ("401 Unauthorized", 401),
        ("500 Internal Server Error", 500),
    ] {
        let (result, _) = exchange(&http_response(status, "", b"{\"success\":false}"));

        assert_eq!(
            result,
            Err(TransportError::BadResponse {
                reason: format!("HTTP {code}")
            }),
            "{status}"
        );
    }
}

/// Pins ureq's `max_redirects(0)`: the 3xx is returned, not followed, so the bearer token never
/// reaches the `Location` (TST-18).
#[test]
#[cfg_attr(miri, ignore = "opens loopback sockets")]
fn a_redirect_is_not_followed_and_is_a_bad_response() {
    let elsewhere = TcpListener::bind("127.0.0.1:0").expect("a second loopback port");
    elsewhere
        .set_nonblocking(true)
        .expect("a non-blocking listener");
    let location = format!(
        "location: {}\r\n",
        url(elsewhere.local_addr().expect("its address"))
    );

    for status in [
        "302 Found",
        "307 Temporary Redirect",
        "308 Permanent Redirect",
    ] {
        let (result, _) = exchange(&http_response(status, &location, b""));

        assert!(
            matches!(&result, Err(TransportError::BadResponse { reason }) if reason.starts_with("HTTP 30")),
            "{status}: {result:?}"
        );
    }
    let followed = elsewhere.accept().map(|_| ());
    assert_eq!(
        followed.map_err(|error| error.kind()),
        Err(io::ErrorKind::WouldBlock),
        "the redirect target saw no connection"
    );
}

/// The status line and headers arrive, then the body stalls with the connection held open:
/// the request's timeout still ends the call (ureq's global timeout covers the body read).
#[test]
#[cfg_attr(miri, ignore = "opens loopback sockets")]
fn a_body_that_stalls_after_the_headers_times_out() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let addr = listener.local_addr().expect("its address");
    let token = token();
    let short = Duration::from_millis(200);

    let (result, elapsed) = thread::scope(|scope| {
        let server = thread::Builder::new()
            .name("stalling-workers-ai".to_owned())
            .spawn_scoped(scope, || {
                let (_, mut stream) = serve_and_hold(
                    &listener,
                    b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 100\r\n\r\n{\"res",
                );
                // Holds the connection until the client gives up and closes it (or the
                // fake's own read timeout passes); what it reads is not the test's concern.
                let mut rest = Vec::new();
                let held: io::Result<usize> = stream.read_to_end(&mut rest);
                drop(held);
            })
            .expect("a server thread");
        let start = Instant::now();
        let result = UreqTransport::new().send(&request(&url(addr), &token, short));
        let elapsed = start.elapsed();
        TcpStream::connect(addr).expect("a dummy connection");
        server.join().expect("the stalling server");
        (result, elapsed)
    });

    assert_eq!(result, Err(TransportError::TimedOut { after: short }));
    assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
}

#[test]
#[cfg_attr(miri, ignore = "opens loopback sockets")]
fn a_server_that_never_answers_times_out_within_the_request_timeout() {
    // Accepted by the kernel's backlog and never read: the request goes out, no reply comes.
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let addr = listener.local_addr().expect("its address");
    let token = token();
    let short = Duration::from_millis(300);

    let result = UreqTransport::new().send(&request(&url(addr), &token, short));

    assert_eq!(result, Err(TransportError::TimedOut { after: short }));
    drop(listener);
}

#[test]
#[cfg_attr(miri, ignore = "opens loopback sockets")]
fn a_refused_connection_is_unreachable() {
    let token = token();

    // Port 0 cannot be connected to on any platform, so the refusal is immediate.
    let result = UreqTransport::new().send(&request(
        "http://127.0.0.1:0/client/v4/accounts/a/ai/run/m",
        &token,
        TIMEOUT,
    ));

    assert!(
        matches!(&result, Err(TransportError::Unreachable { reason }) if reason.starts_with("connection failed")),
        "{result:?}"
    );
}

#[test]
#[cfg_attr(miri, ignore = "opens loopback sockets")]
fn an_unparseable_response_is_a_bad_response() {
    let (result, _) = exchange(b"not http at all\r\n\r\n");

    assert!(
        matches!(result, Err(TransportError::BadResponse { .. })),
        "{result:?}"
    );
}

#[test]
#[cfg_attr(miri, ignore = "opens loopback sockets")]
fn no_failure_shows_the_token_or_the_url() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let token = token();
    let silent = url(listener.local_addr().expect("its address"));
    let failures = [
        UreqTransport::new().send(&request(&silent, &token, Duration::from_millis(100))),
        UreqTransport::new().send(&request("http://127.0.0.1:0/x", &token, TIMEOUT)),
        UreqTransport::new().send(&request("not a url", &token, TIMEOUT)),
        exchange(&http_response("403 Forbidden", "", SECRET.as_bytes())).0,
    ];

    for failure in failures {
        let error = failure.expect_err("each call fails");
        for shown in [error.to_string(), format!("{error:?}")] {
            assert!(!shown.contains(SECRET), "{shown}");
            assert!(!shown.contains("acct0123"), "{shown}");
            assert!(!shown.contains("127.0.0.1"), "{shown}");
        }
    }
}

/// Pins the agent's configuration: no proxy from `ALL_PROXY`/`HTTPS_PROXY`/`HTTP_PROXY` (ureq's
/// default reads them), no redirects, and 4xx/5xx as responses so the adapter maps them.
#[test]
fn the_agent_takes_no_proxy_follows_no_redirect_and_returns_error_statuses() {
    let transport = UreqTransport::new();
    let config = transport.agent.config();

    assert_eq!(config.proxy().map(|proxy| proxy.uri().to_string()), None);
    assert_eq!(config.max_redirects(), 0);
    assert!(!config.http_status_as_error());
}
