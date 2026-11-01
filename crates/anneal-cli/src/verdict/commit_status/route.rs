//! Where the commit status goes (weigh-in wi-077990 A): the proxy and the trust roots the
//! environment names, read once at the binary edge ([`Network::read`], from `GithubEnv::read`)
//! and turned into a [`Route`] only when a status is about to be sent, so a run without a token
//! reads no file.
//!
//! - **Proxy.** The first non-empty of [`PROXY_VARS`]: `HTTPS_PROXY`, `https_proxy`,
//!   `ALL_PROXY`, `all_proxy` (the API is `https://`, so `HTTP_PROXY` is not read). It must be
//!   an `http://` or `https://` proxy, reached with `CONNECT`; a value without a scheme is
//!   `http://`. Any other value, SOCKS included (ureq is built without `socks-proxy`), is
//!   [`PostError::Proxy`], which names the variable and never the value: it may hold
//!   credentials. The hosts in `NO_PROXY` (else `no_proxy`), comma-separated with ureq's
//!   patterns (`host`, `.suffix`, `*.suffix`, `*`), and loopback ([`LOOPBACK`]) are reached
//!   directly. With no proxy variable the post is direct, as before.
//! - **Roots.** With `SSL_CERT_FILE` set, the PEM certificates in that file are the roots the
//!   API's certificate must chain to, in place of the bundled Mozilla roots: ureq takes one set
//!   of roots and cannot add to its bundled ones, and the file is the system's chosen bundle
//!   (a TLS-terminating proxy's bundle carries the public roots as well). The file is read
//!   within [`CERT_FILE_BYTES`] (H-13). A file that cannot be read, is over that cap, holds a
//!   malformed PEM section or no certificate fails the post, naming the variable and the path;
//!   verification is never dropped. A certificate rustls cannot parse is skipped by rustls, so
//!   a bundle of only such entries trusts nothing and every TLS handshake fails. Without
//!   `SSL_CERT_FILE` the roots are the bundled ones, as before.

use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::File;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use ureq::tls::{PemItem, RootCerts};
use ureq::{Proxy, ProxyProtocol};

use super::PostError;

/// The variables a proxy is read from, in order.
const PROXY_VARS: [&str; 4] = ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"];

/// The variables the hosts reached without the proxy are read from, in order.
const NO_PROXY_VARS: [&str; 2] = ["NO_PROXY", "no_proxy"];

/// The variable naming the PEM bundle of trust roots.
pub(super) const CERT_FILE_VAR: &str = "SSL_CERT_FILE";

/// Most KiB read of the [`CERT_FILE_VAR`] file.
pub(super) const CERT_FILE_KIB: u64 = 4096;

/// Most bytes read of the [`CERT_FILE_VAR`] file: a system bundle is a few hundred KiB.
pub(super) const CERT_FILE_BYTES: u64 = CERT_FILE_KIB * 1024;

/// Hosts always reached without the proxy: a proxy elsewhere cannot reach this machine's
/// loopback (only the debug-build test API, `ANNEAL_GITHUB_API`, is ever there).
const LOOPBACK: [&str; 4] = ["localhost", "127.0.0.1", "::1", "[::1]"];

/// What the environment says about the way to GitHub. Its `Debug` names the proxy's variable,
/// never its value.
#[derive(Default)]
pub(super) struct Network {
    /// The proxy's variable and value.
    proxy: Option<(&'static str, OsString)>,
    /// The hosts reached without the proxy, as the variable gave them.
    no_proxy: Option<OsString>,
    /// The PEM bundle of trust roots.
    cert_file: Option<PathBuf>,
}

impl fmt::Debug for Network {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Network")
            .field("proxy_var", &self.proxy.as_ref().map(|(var, _)| var))
            .field("no_proxy", &self.no_proxy)
            .field("cert_file", &self.cert_file)
            .finish()
    }
}

impl Network {
    /// The first non-empty of [`PROXY_VARS`] and of [`NO_PROXY_VARS`], and [`CERT_FILE_VAR`]
    /// when non-empty.
    pub(super) fn read(lookup: impl Fn(&str) -> Option<OsString>) -> Self {
        let set = |var: &str| lookup(var).filter(|value| !value.is_empty());
        Self {
            proxy: PROXY_VARS
                .into_iter()
                .find_map(|var| set(var).map(|value| (var, value))),
            no_proxy: NO_PROXY_VARS.into_iter().find_map(&set),
            cert_file: set(CERT_FILE_VAR).map(PathBuf::from),
        }
    }

    /// The route the environment names: its proxy, if any, and its roots.
    ///
    /// # Errors
    /// [`PostError::Proxy`] for a proxy that is not `http://` or `https://`, and the
    /// `CertFile*` variants for a [`CERT_FILE_VAR`] file that gives no roots.
    pub(super) fn route(&self) -> Result<Route, PostError> {
        let proxy = self
            .proxy
            .as_ref()
            .map(|(var, value)| proxy(var, value, self.no_proxy.as_deref()))
            .transpose()?;
        let roots = match &self.cert_file {
            None => RootCerts::WebPki,
            Some(path) => roots(path)?,
        };
        Ok(Route { proxy, roots })
    }
}

/// The proxy, if any, and the trust roots one status is sent with.
#[derive(Debug)]
pub(super) struct Route {
    /// The `CONNECT` proxy, with the hosts it is not used for.
    pub(super) proxy: Option<Proxy>,
    /// The roots the server's certificate must chain to.
    pub(super) roots: RootCerts,
}

#[cfg(test)]
impl Route {
    /// No proxy and the bundled roots: what an environment without the variables gives.
    pub(super) const fn direct() -> Self {
        Self {
            proxy: None,
            roots: RootCerts::WebPki,
        }
    }
}

/// The `http://` or `https://` proxy `value` of `var`, bypassed for loopback and the hosts in
/// `no_proxy`.
///
/// # Errors
/// [`PostError::Proxy`] when `value` is not such a proxy.
fn proxy(var: &'static str, value: &OsStr, no_proxy: Option<&OsStr>) -> Result<Proxy, PostError> {
    let refused = |_unusable| PostError::Proxy { var };
    let parsed = value
        .to_str()
        .ok_or(PostError::Proxy { var })
        .and_then(|value| Proxy::new(value).map_err(refused))?;
    let protocol = parsed.protocol();
    if !matches!(protocol, ProxyProtocol::Http | ProxyProtocol::Https) {
        return Err(PostError::Proxy { var });
    }
    // ureq adds `NO_PROXY` hosts only through its builder (or from the process's environment,
    // which is not read here), so the parsed proxy is built again with them.
    let mut builder = Proxy::builder(protocol)
        .host(parsed.host())
        .port(parsed.port());
    if let Some(username) = parsed.username() {
        builder = builder.username(username);
    }
    if let Some(password) = parsed.password() {
        builder = builder.password(password);
    }
    // A host that is not UTF-8 matches no host in an API URL, so a lossy reading changes
    // nothing that could be bypassed.
    let listed = no_proxy.map_or(Cow::Borrowed(""), OsStr::to_string_lossy);
    let hosts = LOOPBACK
        .into_iter()
        .chain(listed.split(',').map(str::trim))
        .filter(|host| !host.is_empty());
    for host in hosts {
        builder = builder.no_proxy(host);
    }
    builder.build().map_err(refused)
}

/// The certificates of the PEM bundle at `path`, as the only roots.
///
/// # Errors
/// [`PostError::CertFileUnreadable`], [`PostError::CertFileTooLarge`],
/// [`PostError::CertFileMalformed`] or [`PostError::CertFileEmpty`].
fn roots(path: &Path) -> Result<RootCerts, PostError> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| {
            file.take(CERT_FILE_BYTES.saturating_add(1))
                .read_to_end(&mut bytes)
        })
        .map_err(|source| PostError::CertFileUnreadable {
            path: path.to_path_buf(),
            source,
        })?;
    if u64::try_from(bytes.len()).map_or(true, |len| len > CERT_FILE_BYTES) {
        return Err(PostError::CertFileTooLarge {
            path: path.to_path_buf(),
        });
    }
    let mut certs = Vec::new();
    for item in ureq::tls::parse_pem(&bytes) {
        match item {
            Ok(PemItem::Certificate(cert)) => certs.push(cert),
            // A private key or another item is no root; a bundle may carry one.
            Ok(_) => {}
            // The section is named by the failure; ureq's message adds nothing for the user.
            Err(_malformed) => {
                return Err(PostError::CertFileMalformed {
                    path: path.to_path_buf(),
                });
            }
        }
    }
    if certs.is_empty() {
        return Err(PostError::CertFileEmpty {
            path: path.to_path_buf(),
        });
    }
    Ok(RootCerts::from(certs))
}

#[cfg(test)]
mod tests;
