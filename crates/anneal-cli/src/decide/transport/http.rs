//! The Workers AI adapter of [`Transport`](super::Transport), over ureq with rustls and the
//! bundled Mozilla roots.
//!
//! One blocking `POST` per call, bounded by the request's timeout from DNS to the last body
//! byte (H-13). The agent follows no redirect (a 3xx is a [`TransportError::BadResponse`], so
//! the bearer token never goes to another host), takes no proxy from the environment, and
//! reads at most [`MAX_RESPONSE_BYTES`] of body. No error carries the token, the URL or a
//! ureq message: each failure maps to a fixed reason.

use std::io::{self, Read as _};
use std::time::Duration;

use super::{ClefRequest, MAX_RESPONSE_BYTES, Transport, TransportError};

/// Bytes read of a response body: one past [`MAX_RESPONSE_BYTES`], so a larger body shows.
const READ_CAP: u64 = 64 * 1024 + 1;

/// The `User-Agent` header.
const USER_AGENT: &str = concat!("anneal/", env!("CARGO_PKG_VERSION"));

/// Sends Clef's requests over HTTP(S) with ureq.
#[derive(Debug)]
pub(crate) struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    /// An adapter with no proxy, no redirects and error statuses returned as responses.
    pub(crate) fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .proxy(None)
            .max_redirects(0)
            .http_status_as_error(false)
            .user_agent(USER_AGENT)
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }
}

impl Transport for UreqTransport {
    fn send(&self, request: &ClefRequest<'_>) -> Result<Vec<u8>, TransportError> {
        let after = request.timeout;
        let bearer = format!("Bearer {}", request.token.expose_secret());
        let mut response = self
            .agent
            .post(request.url.as_str())
            .config()
            .timeout_global(Some(after))
            .build()
            .header("authorization", bearer.as_str())
            .content_type("application/json")
            .send(request.body.as_slice())
            .map_err(|error| failure(&error, after))?;
        let status = response.status();
        if !status.is_success() {
            return Err(bad(format!("HTTP {}", status.as_u16())));
        }
        let mut body = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(READ_CAP)
            .read_to_end(&mut body)
            .map_err(|error| failure(&ureq::Error::from(error), after))?;
        if body.len() > MAX_RESPONSE_BYTES {
            return Err(bad(format!(
                "response body over {MAX_RESPONSE_BYTES} bytes"
            )));
        }
        Ok(body)
    }
}

/// A bad response for `reason`.
const fn bad(reason: String) -> TransportError {
    TransportError::BadResponse { reason }
}

/// Unreachable for `reason`.
fn unreachable(reason: &str) -> TransportError {
    TransportError::Unreachable {
        reason: reason.to_owned(),
    }
}

/// The failure for ureq's `error`, with a fixed reason: ureq's own messages can hold the URL.
fn failure(error: &ureq::Error, after: Duration) -> TransportError {
    match error {
        ureq::Error::Timeout(_) => TransportError::TimedOut { after },
        ureq::Error::Io(io) if io.kind() == io::ErrorKind::TimedOut => {
            TransportError::TimedOut { after }
        }
        ureq::Error::Io(io) => unreachable(&format!("connection failed ({})", io.kind())),
        ureq::Error::HostNotFound => unreachable("host not found"),
        ureq::Error::ConnectionFailed => unreachable("connection failed"),
        ureq::Error::Tls(_)
        | ureq::Error::Pem(_)
        | ureq::Error::Rustls(_)
        | ureq::Error::TlsRequired => unreachable("TLS failure"),
        ureq::Error::BadUri(_) | ureq::Error::Http(_) => unreachable("invalid request"),
        ureq::Error::Protocol(_) | ureq::Error::LargeResponseHeader(..) => {
            bad("malformed HTTP response".to_owned())
        }
        ureq::Error::BodyExceedsLimit(_) => {
            bad(format!("response body over {MAX_RESPONSE_BYTES} bytes"))
        }
        ureq::Error::StatusCode(code) => bad(format!("HTTP {code}")),
        ureq::Error::TooManyRedirects | ureq::Error::RedirectFailed => {
            bad("redirect refused".to_owned())
        }
        _ => unreachable("request failed"),
    }
}

#[cfg(test)]
mod tests;
