use std::io;

use super::UreqFailure;

#[test]
fn a_timeout_from_ureq_or_from_the_socket_is_timed_out() {
    for error in [
        ureq::Error::Timeout(ureq::Timeout::Global),
        ureq::Error::Io(io::Error::from(io::ErrorKind::TimedOut)),
    ] {
        assert_eq!(UreqFailure::of(&error), UreqFailure::TimedOut, "{error:?}");
    }
}

#[test]
fn connection_failures_keep_the_io_kind_when_there_is_one() {
    assert_eq!(
        UreqFailure::of(&ureq::Error::Io(io::Error::from(
            io::ErrorKind::ConnectionRefused
        ))),
        UreqFailure::ConnectionFailed {
            io: Some(io::ErrorKind::ConnectionRefused)
        }
    );
    assert_eq!(
        UreqFailure::of(&ureq::Error::ConnectionFailed),
        UreqFailure::ConnectionFailed { io: None }
    );
}

#[test]
fn a_uri_or_request_ureq_could_not_build_is_an_invalid_request() {
    let http = ureq::http::Request::builder()
        .uri("http://[not a host")
        .body(())
        .expect_err("an unparsable URI is an http::Error");
    for error in [
        ureq::Error::BadUri("no scheme".to_owned()),
        ureq::Error::Http(http),
    ] {
        assert_eq!(
            UreqFailure::of(&error),
            UreqFailure::InvalidRequest,
            "{error:?}"
        );
    }
}

#[test]
fn every_other_named_kind_has_its_own_class() {
    let cases = [
        (ureq::Error::HostNotFound, UreqFailure::HostNotFound),
        (ureq::Error::TlsRequired, UreqFailure::Tls),
        (ureq::Error::Tls("handshake"), UreqFailure::Tls),
        (
            ureq::Error::LargeResponseHeader(1, 2),
            UreqFailure::MalformedResponse,
        ),
        (ureq::Error::BodyExceedsLimit(9), UreqFailure::BodyTooLarge),
        (ureq::Error::StatusCode(503), UreqFailure::Status(503)),
        (ureq::Error::TooManyRedirects, UreqFailure::Redirect),
        (ureq::Error::RedirectFailed, UreqFailure::Redirect),
        (ureq::Error::InvalidProxyUrl, UreqFailure::Other),
    ];
    for (error, class) in cases {
        assert_eq!(UreqFailure::of(&error), class, "{error:?}");
    }
}
