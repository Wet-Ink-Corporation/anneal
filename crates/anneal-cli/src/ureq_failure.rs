//! What a failed ureq call means, shared by the two ureq adapters (Clef's transport,
//! `decide::transport::http`, and the commit status client, `verdict::commit_status::http`).
//!
//! [`UreqFailure`] keeps only the kind of failure, never ureq's message, which can hold the
//! URL; each adapter maps it to its own error with a fixed reason.

use std::io;

/// The kind of a failed ureq call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UreqFailure {
    /// No answer within the call's timeout, by ureq's clock or the socket's.
    TimedOut,
    /// The connection failed or broke, with the I/O error's kind when there was one.
    ConnectionFailed {
        /// The kind of the I/O error.
        io: Option<io::ErrorKind>,
    },
    /// The host's name did not resolve.
    HostNotFound,
    /// TLS failed, or was required and not used.
    Tls,
    /// ureq could not build the request (`BadUri`, `Http`): nothing was sent.
    InvalidRequest,
    /// The answer was not HTTP that could be read.
    MalformedResponse,
    /// The body was over ureq's limit.
    BodyTooLarge,
    /// ureq turned an answer's status into an error.
    Status(u16),
    /// A redirect was refused or failed.
    Redirect,
    /// Anything else, including kinds a later ureq adds.
    Other,
}

impl UreqFailure {
    /// The kind of ureq's `error`.
    ///
    /// ureq's `#[doc(hidden)]` `BodyStalled`, which its docs say is never seen (a stalled body
    /// surfaces as [`ureq::Error::Timeout`]), is not matched by name; it is [`Self::Other`].
    pub(crate) fn of(error: &ureq::Error) -> Self {
        match error {
            ureq::Error::Timeout(_) => Self::TimedOut,
            ureq::Error::Io(io) if io.kind() == io::ErrorKind::TimedOut => Self::TimedOut,
            ureq::Error::Io(io) => Self::ConnectionFailed {
                io: Some(io.kind()),
            },
            ureq::Error::ConnectionFailed => Self::ConnectionFailed { io: None },
            ureq::Error::HostNotFound => Self::HostNotFound,
            ureq::Error::Tls(_)
            | ureq::Error::Pem(_)
            | ureq::Error::Rustls(_)
            | ureq::Error::TlsRequired => Self::Tls,
            ureq::Error::BadUri(_) | ureq::Error::Http(_) => Self::InvalidRequest,
            ureq::Error::Protocol(_) | ureq::Error::LargeResponseHeader(..) => {
                Self::MalformedResponse
            }
            ureq::Error::BodyExceedsLimit(_) => Self::BodyTooLarge,
            ureq::Error::StatusCode(code) => Self::Status(*code),
            ureq::Error::TooManyRedirects | ureq::Error::RedirectFailed => Self::Redirect,
            _ => Self::Other,
        }
    }
}

#[cfg(test)]
mod tests;
