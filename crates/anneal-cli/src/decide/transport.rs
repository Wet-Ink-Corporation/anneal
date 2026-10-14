//! The port to Workers AI: [`Transport`] sends one [`ClefRequest`] and returns the response
//! body, or a [`TransportError`] that says whether Clef was unreachable, timed out or answered
//! badly.
//!
//! Production sends through `http::UreqTransport` (ureq with rustls); the test-only
//! `NoHttpClient` never sends. An adapter must keep the [`ApiToken`] out of
//! every error it returns, honour the request's timeout, and refuse a body over
//! [`MAX_RESPONSE_BYTES`].

pub(crate) mod http;

use std::fmt;
use std::time::Duration;

use serde::Serialize;

/// The largest response body an adapter accepts; a larger one is [`TransportError::BadResponse`]
/// (H-13).
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// Why every call through [`NoHttpClient`] fails. Test-only: the wording predates the HTTP
/// client and is pinned by `without_an_http_client_every_call_is_unreachable`.
#[cfg(test)]
pub const NO_HTTP_CLIENT: &str = "no HTTP client configured (pending dependency decision)";

/// The Workers AI API token, which never formats.
///
/// Its `Debug` is redacted and it has no `Display`, so it cannot reach a log row, an error or
/// an envelope by formatting; only an adapter reads it, through [`ApiToken::expose_secret`].
/// It is not `Clone`, so the configuration holds the one copy.
pub struct ApiToken(String);

impl ApiToken {
    /// A token, if it is fit for an `Authorization` header: non-empty printable ASCII with no
    /// whitespace. The value is never part of the refusal.
    #[must_use]
    pub fn new(value: String) -> Option<Self> {
        let valid = !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_graphic());
        valid.then_some(Self(value))
    }

    /// The secret, for the adapter's `Authorization: Bearer` header and nothing else.
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiToken(<redacted>)")
    }
}

/// One call to Workers AI: `POST url` with `Authorization: Bearer <token>` and the JSON
/// `body`, given at most `timeout`.
#[derive(Debug)]
pub struct ClefRequest<'a> {
    /// `https://api.cloudflare.com/client/v4/accounts/{account}/ai/run/{model}`.
    pub url: String,
    /// The model id in the URL, for the adapter's diagnostics.
    pub model: &'a str,
    /// The JSON request body.
    pub body: Vec<u8>,
    /// The bearer token.
    pub token: &'a ApiToken,
    /// The whole call's deadline: connect, send and read.
    pub timeout: Duration,
}

/// Why a call produced no usable response. No variant may carry the token.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    /// Clef could not be reached: no client, no token, DNS, connect or TLS failure.
    #[error("Clef unreachable: {reason}")]
    Unreachable {
        /// What failed, without the token.
        reason: String,
    },
    /// No complete response within the timeout.
    #[error("Clef timed out after {} ms", after.as_millis())]
    TimedOut {
        /// The timeout that passed.
        after: Duration,
    },
    /// Clef answered, but not with a usable response (a non-2xx status, an oversized or
    /// unreadable body, a missing probability).
    #[error("Clef gave a bad response: {reason}")]
    BadResponse {
        /// What was wrong, without the token.
        reason: String,
    },
}

/// The kind of a failed call, as a log row and an envelope record it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// [`TransportError::Unreachable`].
    Unreachable,
    /// [`TransportError::TimedOut`].
    TimedOut,
    /// [`TransportError::BadResponse`].
    BadResponse,
}

impl TransportError {
    /// The kind of failure.
    #[must_use]
    pub const fn kind(&self) -> FailureKind {
        match self {
            Self::Unreachable { .. } => FailureKind::Unreachable,
            Self::TimedOut { .. } => FailureKind::TimedOut,
            Self::BadResponse { .. } => FailureKind::BadResponse,
        }
    }
}

/// Sends one request to Workers AI.
pub trait Transport {
    /// Sends `request` and returns the body of a 2xx response.
    ///
    /// # Errors
    /// [`TransportError::Unreachable`] when the endpoint cannot be reached,
    /// [`TransportError::TimedOut`] past `request.timeout`, [`TransportError::BadResponse`] for
    /// a non-2xx status or a body over [`MAX_RESPONSE_BYTES`].
    fn send(&self, request: &ClefRequest<'_>) -> Result<Vec<u8>, TransportError>;
}

/// A test transport that never sends: every call is unreachable. The binary uses
/// `http::UreqTransport`.
#[cfg(test)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NoHttpClient;

#[cfg(test)]
impl Transport for NoHttpClient {
    fn send(&self, _request: &ClefRequest<'_>) -> Result<Vec<u8>, TransportError> {
        Err(TransportError::Unreachable {
            reason: NO_HTTP_CLIENT.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{ApiToken, ClefRequest, NoHttpClient, Transport, TransportError};

    const SECRET: &str = "s3cr3t-T0KEN-value";

    fn token() -> ApiToken {
        ApiToken::new(SECRET.to_owned()).expect("a printable token")
    }

    #[test]
    fn the_token_debugs_as_redacted_and_exposes_its_value_only_on_request() {
        let token = token();

        assert_eq!(format!("{token:?}"), "ApiToken(<redacted>)");
        assert_eq!(token.expose_secret(), SECRET, "the adapter's bearer value");
    }

    #[test]
    fn a_request_debug_shows_no_token() {
        let token = token();
        let request = ClefRequest {
            url: "https://api.example/run/m".to_owned(),
            model: "m",
            body: b"{}".to_vec(),
            token: &token,
            timeout: Duration::from_secs(2),
        };

        let shown = format!("{request:?}");

        assert!(!shown.contains(SECRET), "{shown}");
        assert!(shown.contains("<redacted>"), "{shown}");
    }

    #[test]
    fn a_token_with_whitespace_or_control_characters_is_refused() {
        for bad in ["", "a b", "a\nb", "tab\there"] {
            assert!(
                ApiToken::new(bad.to_owned()).is_none(),
                "{bad:?} is refused"
            );
        }
    }

    #[test]
    fn without_an_http_client_every_call_is_unreachable() {
        let token = token();
        let request = ClefRequest {
            url: "https://api.example/run/m".to_owned(),
            model: "m",
            body: Vec::new(),
            token: &token,
            timeout: Duration::from_secs(2),
        };

        let result = NoHttpClient.send(&request);

        assert!(matches!(
            result,
            Err(TransportError::Unreachable { reason })
                if reason == "no HTTP client configured (pending dependency decision)"
        ));
    }
}
