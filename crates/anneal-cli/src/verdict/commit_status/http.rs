//! The ureq adapter that sends one commit status: `POST
//! <api>/repos/<owner>/<repo>/statuses/<sha>` with `{state, context, description}`.
//!
//! The call is bounded by the client's timeout from DNS to the last byte (H-13). As Clef's
//! transport (`decide::transport::http`), the agent follows no redirect (a 3xx is a refusal, so
//! the token never goes to another host) and takes no proxy from the environment. A refusal's
//! body is read through a cap of [`ERROR_BODY_BYTES`] for GitHub's `message`, which is shown
//! printable and cut to [`MESSAGE_CHARS`]; no error carries the token, the URL or a ureq
//! message. ureq's resolver thread may outlive a call that times out, as documented there; a
//! `verdict set` process exits soon after.

use std::io::{self, Read as _};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{CONTEXT, PostError, State};
use crate::decide::transport::ApiToken;

/// Bytes read of a refusal's body: more, and its message is not shown.
const ERROR_BODY_BYTES: u64 = 16 * 1024;

/// Characters of GitHub's `message` shown.
const MESSAGE_CHARS: usize = 120;

/// The `User-Agent` header (GitHub refuses a request without one).
const USER_AGENT: &str = concat!("anneal/", env!("CARGO_PKG_VERSION"));

/// The REST API version asked for.
const API_VERSION: &str = "2022-11-28";

/// One status to send.
#[derive(Debug)]
pub(crate) struct Request<'a> {
    /// `<api>/repos/<owner>/<repo>/statuses/<sha>`.
    pub(crate) url: String,
    /// The bearer token.
    pub(crate) token: &'a ApiToken,
    /// The state.
    pub(crate) state: State,
    /// The description, at most 140 characters.
    pub(crate) description: &'a str,
}

/// The JSON body GitHub reads.
#[derive(Debug, Serialize)]
struct Body<'a> {
    state: State,
    context: &'static str,
    description: &'a str,
}

/// The part of a refusal's body shown.
#[derive(Debug, Deserialize)]
struct Refusal {
    message: String,
}

/// Sends commit statuses with ureq.
#[derive(Debug)]
pub(crate) struct Client {
    agent: ureq::Agent,
    timeout: Duration,
}

impl Client {
    /// A client whose every call ends within `timeout`, with no proxy, no redirects and error
    /// statuses returned as responses.
    pub(crate) fn new(timeout: Duration) -> Self {
        let config = ureq::Agent::config_builder()
            .proxy(None)
            .max_redirects(0)
            .http_status_as_error(false)
            .user_agent(USER_AGENT)
            .timeout_global(Some(timeout))
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
            timeout,
        }
    }

    /// Sends `request`.
    ///
    /// # Errors
    /// The [`PostError`] for a call that failed or a status other than 2xx.
    pub(crate) fn post(&self, request: &Request<'_>) -> Result<(), PostError> {
        let body = serde_json::to_vec(&Body {
            state: request.state,
            context: CONTEXT,
            description: request.description,
        })
        .map_err(PostError::Body)?;
        let bearer = format!("Bearer {}", request.token.expose_secret());
        let mut response = self
            .agent
            .post(request.url.as_str())
            .header("authorization", bearer.as_str())
            .header("accept", "application/vnd.github+json")
            .header("x-github-api-version", API_VERSION)
            .content_type("application/json")
            .send(body.as_slice())
            .map_err(|error| self.failure(&error))?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let mut text = Vec::new();
        let read = response
            .body_mut()
            .as_reader()
            .take(ERROR_BODY_BYTES.saturating_add(1))
            .read_to_end(&mut text);
        let detail = match read {
            Ok(_) => message(&text).map_or_else(String::new, |message| format!(": {message}")),
            // The status code is the failure reported; a body that cannot be read only leaves
            // GitHub's explanation out.
            Err(_unread) => String::new(),
        };
        Err(PostError::Status {
            code: status.as_u16(),
            detail,
        })
    }

    /// The failure for ureq's `error`, with a fixed reason: ureq's own messages can hold the
    /// URL.
    fn failure(&self, error: &ureq::Error) -> PostError {
        let unreachable = |reason| PostError::Unreachable { reason };
        match error {
            ureq::Error::Timeout(_) => PostError::TimedOut {
                after: self.timeout,
            },
            ureq::Error::Io(io) if io.kind() == io::ErrorKind::TimedOut => PostError::TimedOut {
                after: self.timeout,
            },
            ureq::Error::Io(_) | ureq::Error::ConnectionFailed => {
                unreachable("the connection failed")
            }
            ureq::Error::HostNotFound => unreachable("the host was not found"),
            ureq::Error::Tls(_)
            | ureq::Error::Pem(_)
            | ureq::Error::Rustls(_)
            | ureq::Error::TlsRequired => unreachable("TLS failed"),
            ureq::Error::StatusCode(code) => PostError::Status {
                code: *code,
                detail: String::new(),
            },
            _ => PostError::BadResponse,
        }
    }
}

/// GitHub's `message` in a refusal's `body` (at most [`ERROR_BODY_BYTES`]), with control
/// characters dropped and cut to [`MESSAGE_CHARS`]; `None` when there is none to show.
fn message(body: &[u8]) -> Option<String> {
    let over_cap = u64::try_from(body.len()).map_or(true, |len| len > ERROR_BODY_BYTES);
    if over_cap {
        return None;
    }
    // A body that is not GitHub's error JSON has no message to show; the code still reports
    // the refusal.
    let refusal: Refusal = serde_json::from_slice(body).ok()?;
    let shown: String = refusal
        .message
        .chars()
        .filter(|c| !c.is_control())
        .take(MESSAGE_CHARS)
        .collect();
    Some(shown).filter(|shown| !shown.trim().is_empty())
}

#[cfg(test)]
mod tests;
