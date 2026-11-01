//! The commit status client going the way the environment names: through a `CONNECT` proxy,
//! or directly to loopback with one set. The fakes run on loopback ports; nothing reaches the
//! network.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use crate::decide::transport::ApiToken;
use crate::verdict::commit_status::http::{Client, Request};
use crate::verdict::commit_status::route::Network;
use crate::verdict::commit_status::{PostError, State};

const CREATED: &[u8] = b"HTTP/1.1 201 Created\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}";

/// A lookup with `HTTPS_PROXY` set to `proxy` and nothing else.
fn with_proxy(proxy: String) -> impl Fn(&str) -> Option<OsString> {
    move |var| (var == "HTTPS_PROXY").then(|| OsString::from(&proxy))
}

/// One request's head, up to the blank line, reading its body by its `content-length`.
fn read_request(reader: &mut impl BufRead) -> String {
    let mut head = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).expect("a header line") == 0 || line == "\r\n" {
            break;
        }
        head.push_str(&line);
    }
    let length: usize = head
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().expect("a numeric content-length"))
        })
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).expect("the declared body");
    head
}

/// A `CONNECT` proxy that opens the tunnel and answers inside it as GitHub would: returns
/// the `CONNECT` request's head and the tunnelled request's head.
fn proxy_one(listener: &TcpListener) -> (String, String) {
    let (stream, _) = listener.accept().expect("a connection");
    let mut reader = BufReader::new(stream.try_clone().expect("a second handle"));
    let connect = read_request(&mut reader);
    if !connect.starts_with("CONNECT ") {
        return (connect, String::new());
    }
    let mut writer = stream;
    writer
        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
        .expect("the tunnel is opened");
    let tunnelled = read_request(&mut reader);
    writer.write_all(CREATED).expect("the status is created");
    (connect, tunnelled)
}

/// A fake GitHub answering one request: returns its head.
fn github_one(listener: &TcpListener) -> String {
    let (stream, _) = listener.accept().expect("a connection");
    let mut reader = BufReader::new(stream.try_clone().expect("a second handle"));
    let head = read_request(&mut reader);
    let mut writer = stream;
    // When the client never came this is the dummy connection, which may refuse the answer;
    // the client's own result shows whether a real request got it.
    let wrote: std::io::Result<()> = writer.write_all(CREATED);
    drop(wrote);
    head
}

/// Sends one status to `url` through `client` while `serve` answers on `listener`; returns
/// the post's result and what `serve` saw.
fn post_while<T: Send>(
    client: &Client,
    url: String,
    listener: &TcpListener,
    serve: impl FnOnce(&TcpListener) -> T + Send,
) -> (Result<(), PostError>, T) {
    let addr = listener.local_addr().expect("its address");
    let token = ApiToken::new("t0k".to_owned()).expect("a valid token");
    std::thread::scope(|scope| {
        let server = std::thread::Builder::new()
            .name("fake-server".to_owned())
            .spawn_scoped(scope, || serve(listener))
            .expect("a server thread");
        let request = Request {
            url,
            token: &token,
            state: State::Success,
            description: "d",
        };
        let result = client.post(&request);
        // Unblocks `accept` if the client never connected; otherwise it waits in the backlog.
        TcpStream::connect(addr).expect("a dummy connection");
        (result, server.join().expect("the fake"))
    })
}

#[test]
fn a_status_goes_through_the_proxy_the_environment_names() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let addr = listener.local_addr().expect("its address");
    let route = Network::read(with_proxy(format!("http://{addr}")))
        .route()
        .expect("a route");
    let client = Client::with_route(Duration::from_secs(10), route);
    let url = "http://github.test/repos/owner/repo/statuses/abc123".to_owned();

    let (result, (connect, tunnelled)) = post_while(&client, url, &listener, proxy_one);

    assert!(result.is_ok(), "{result:?}");
    assert!(
        connect.starts_with("CONNECT github.test:80 HTTP/1.1\r\n"),
        "{connect}"
    );
    assert!(
        !connect.contains("t0k"),
        "the proxy is not given the token: {connect}"
    );
    assert!(
        tunnelled.starts_with("POST /repos/owner/repo/statuses/abc123 HTTP/1.1\r\n"),
        "{tunnelled}"
    );
    assert!(
        tunnelled.contains("authorization: Bearer t0k\r\n"),
        "{tunnelled}"
    );
}

#[test]
fn a_loopback_api_is_reached_directly_with_a_proxy_set() {
    // A proxy on a port nothing listens on: a client that used it would find it unreachable.
    let closed = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let dead = closed.local_addr().expect("its address");
    drop(closed);
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let addr = listener.local_addr().expect("its address");
    let route = Network::read(with_proxy(format!("http://{dead}")))
        .route()
        .expect("a route");
    let client = Client::with_route(Duration::from_secs(10), route);
    let url = format!("http://{addr}/repos/owner/repo/statuses/abc123");

    let (result, head) = post_while(&client, url, &listener, github_one);

    assert!(result.is_ok(), "{result:?}");
    assert!(
        head.starts_with("POST /repos/owner/repo/statuses/abc123 HTTP/1.1\r\n"),
        "{head}"
    );
}

#[test]
fn with_no_proxy_variable_the_status_goes_directly() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let addr = listener.local_addr().expect("its address");
    let route = Network::read(|_| None).route().expect("a route");
    let client = Client::with_route(Duration::from_secs(10), route);
    let url = format!("http://{addr}/repos/owner/repo/statuses/abc123");

    let (result, head) = post_while(&client, url, &listener, github_one);

    assert!(result.is_ok(), "{result:?}");
    assert!(head.starts_with("POST /repos/owner/repo/"), "{head}");
}
