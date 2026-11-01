mod through_proxy;

use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use ureq::tls::RootCerts;

use super::{CERT_FILE_BYTES, Network};
use crate::error_chain::reason;
use crate::verdict::commit_status::PostError;
use crate::verdict::commit_status::http::Client;

/// A lookup over `pairs`, as `std::env::var_os` would answer.
fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let pairs: Vec<(String, OsString)> = pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), OsString::from(value)))
        .collect();
    move |var| {
        pairs
            .iter()
            .find(|(key, _)| key == var)
            .map(|(_, value)| value.clone())
    }
}

/// A self-signed P-256 test root (`CN=anneal test root`), trusted by nothing.
const TEST_ROOT: &str = "-----BEGIN CERTIFICATE-----
MIIBjDCCATOgAwIBAgIUXsNASfSzB4BWkYFZI76oQxxUz1wwCgYIKoZIzj0EAwIw
GzEZMBcGA1UEAwwQYW5uZWFsIHRlc3Qgcm9vdDAgFw0yNjExMDEwMDExMjZaGA8y
MTI2MTAwODAwMTEyNlowGzEZMBcGA1UEAwwQYW5uZWFsIHRlc3Qgcm9vdDBZMBMG
ByqGSM49AgEGCCqGSM49AwEHA0IABC9YfYuTTEcgVyvY7zAksW//I5g0vuViesGr
B077BIx53tqeBzD9PseGD/Jz87+AuNNPBFONr8DR4WCgWv7ht+WjUzBRMB0GA1Ud
DgQWBBQH422hhxah5b5kkKERVmV5syIuzDAfBgNVHSMEGDAWgBQH422hhxah5b5k
kKERVmV5syIuzDAPBgNVHRMBAf8EBTADAQH/MAoGCCqGSM49BAMCA0cAMEQCIBE0
+0zJGR4335BgSLU1rEL1x/pTpQ83Ve2tiC00GYRFAiBslna1lF20WTjEhgk3rqZm
fMefColLYlLcldb1gh6Vgw==
-----END CERTIFICATE-----
";

/// The route for a cert file at `path`, with no proxy.
fn route_with_cert_file(path: &Path) -> Result<super::Route, PostError> {
    let path = path.to_str().expect("a UTF-8 temporary path");
    Network::read(lookup(&[("SSL_CERT_FILE", path)])).route()
}

#[test]
fn with_no_proxy_or_cert_variable_the_route_is_direct_with_the_bundled_roots() {
    let route = Network::read(lookup(&[("HTTPS_PROXY", ""), ("SSL_CERT_FILE", "")]))
        .route()
        .expect("a route");

    assert!(route.proxy.is_none(), "{route:?}");
    assert!(matches!(route.roots, RootCerts::WebPki), "{route:?}");
}

#[test]
fn https_proxy_is_taken_before_all_proxy_with_its_credentials() {
    let route = Network::read(lookup(&[
        ("HTTPS_PROXY", ""),
        ("https_proxy", "http://user:p%40ss@10.0.0.1:3128"),
        ("ALL_PROXY", "http://10.0.0.2:3128"),
    ]))
    .route()
    .expect("a route");

    let proxy = route.proxy.expect("a proxy");
    assert_eq!(
        (
            proxy.host(),
            proxy.port(),
            proxy.username(),
            proxy.password()
        ),
        ("10.0.0.1", 3128, Some("user"), Some("p@ss"))
    );
}

#[test]
fn all_proxy_is_taken_when_no_https_proxy_is_set() {
    let route = Network::read(lookup(&[("all_proxy", "https://proxy.example")]))
        .route()
        .expect("a route");

    let proxy = route.proxy.expect("a proxy");
    assert_eq!(
        (proxy.protocol(), proxy.host(), proxy.port()),
        (ureq::ProxyProtocol::Https, "proxy.example", 443)
    );
}

#[test]
fn no_proxy_hosts_and_loopback_bypass_the_proxy() {
    let route = Network::read(lookup(&[
        ("HTTPS_PROXY", "http://10.0.0.1:3128"),
        ("NO_PROXY", "internal.example, .corp.example"),
    ]))
    .route()
    .expect("a route");

    let proxy = route.proxy.expect("a proxy");
    let bypassed = |url: &str| proxy.is_no_proxy(&url.parse().expect("a URI"));
    for direct in [
        "https://internal.example/x",
        "https://git.corp.example/x",
        "http://127.0.0.1:8080/x",
        "http://localhost/x",
        "http://[::1]:8080/x",
    ] {
        assert!(bypassed(direct), "{direct}");
    }
    assert!(!bypassed("https://api.github.com/repos"));
}

#[test]
fn a_socks_or_unparsable_proxy_fails_naming_its_variable_not_its_value() {
    for (var, value) in [
        ("ALL_PROXY", "socks5://user:s3cret@10.0.0.1:1080"),
        ("HTTPS_PROXY", "ftp://user:s3cret@10.0.0.1:21"),
        ("https_proxy", "http://user:s3cret@[not a host"),
    ] {
        let error = Network::read(lookup(&[(var, value)]))
            .route()
            .expect_err("a proxy anneal cannot use");

        assert_eq!(
            reason(&error),
            format!("{var} does not name an http:// or https:// proxy")
        );
    }
}

#[test]
fn a_cert_file_s_certificates_replace_the_bundled_roots() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("bundle.crt");
    let text = format!("a comment before the bundle\n{TEST_ROOT}{TEST_ROOT}");
    std::fs::write(&path, text).expect("the bundle is written");

    let route = route_with_cert_file(&path).expect("a route");

    match route.roots {
        RootCerts::Specific(certs) => assert_eq!(certs.len(), 2),
        other => panic!("expected the file's certificates, got {other:?}"),
    }
}

#[test]
fn a_missing_cert_file_fails_naming_the_variable_and_its_path() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("absent.crt");

    let error = route_with_cert_file(&path).expect_err("no file");

    let reason = reason(&error);
    let expected = format!("SSL_CERT_FILE ({}) cannot be read: ", path.display());
    assert!(reason.starts_with(&expected), "{reason}");
}

#[test]
fn a_cert_file_with_no_certificate_fails() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("garbage.crt");
    std::fs::write(&path, "not a certificate\n").expect("the file is written");

    let error = route_with_cert_file(&path).expect_err("no certificate");

    assert_eq!(
        reason(&error),
        format!(
            "SSL_CERT_FILE ({}) holds no PEM certificate",
            path.display()
        )
    );
}

#[test]
fn a_cert_file_with_a_malformed_pem_section_fails() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("broken.crt");
    let text = format!("{TEST_ROOT}-----BEGIN CERTIFICATE-----\n!!!!\n-----END CERTIFICATE-----\n");
    std::fs::write(&path, text).expect("the file is written");

    let error = route_with_cert_file(&path).expect_err("a malformed section");

    assert_eq!(
        reason(&error),
        format!(
            "SSL_CERT_FILE ({}) holds a malformed PEM section",
            path.display()
        )
    );
}

#[test]
fn a_cert_file_over_the_cap_fails() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("huge.crt");
    let size = usize::try_from(CERT_FILE_BYTES).expect("the cap fits in memory");
    std::fs::write(&path, vec![b'a'; size.saturating_add(1)]).expect("the file is written");

    let error = route_with_cert_file(&path).expect_err("over the cap");

    assert_eq!(
        reason(&error),
        format!("SSL_CERT_FILE ({}) is over 4096 KiB", path.display())
    );
}

#[test]
fn the_client_takes_the_route_s_roots_and_proxy() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("bundle.crt");
    std::fs::write(&path, format!("{TEST_ROOT}{TEST_ROOT}{TEST_ROOT}"))
        .expect("the bundle is written");
    let path = path.to_str().expect("a UTF-8 temporary path");
    let route = Network::read(lookup(&[
        ("SSL_CERT_FILE", path),
        ("HTTPS_PROXY", "http://10.0.0.1:3128"),
    ]))
    .route()
    .expect("a route");

    let client = Client::with_route(Duration::from_secs(10), route);

    let config = client.config();
    match config.tls_config().root_certs() {
        RootCerts::Specific(certs) => assert_eq!(certs.len(), 3),
        other => panic!("expected the file's certificates, got {other:?}"),
    }
    let proxy = config.proxy().expect("the route's proxy");
    assert_eq!((proxy.host(), proxy.port()), ("10.0.0.1", 3128));
}

#[test]
fn the_client_with_no_variable_takes_no_proxy_and_the_bundled_roots() {
    let route = Network::read(lookup(&[])).route().expect("a route");

    let client = Client::with_route(Duration::from_secs(10), route);

    let config = client.config();
    assert!(
        matches!(config.tls_config().root_certs(), RootCerts::WebPki),
        "{:?}",
        config.tls_config().root_certs()
    );
    assert_eq!(config.proxy().map(|proxy| proxy.uri().to_string()), None);
}

#[test]
fn a_cert_file_that_is_a_directory_fails_as_not_a_file() {
    let dir = tempfile::tempdir().expect("a temporary directory");

    let error = route_with_cert_file(dir.path()).expect_err("a directory");

    assert_eq!(
        reason(&error),
        format!("SSL_CERT_FILE ({}) is not a file", dir.path().display())
    );
}

/// A FIFO with no writer would block `open` past the post's timeout: it is refused unopened.
#[cfg(unix)]
#[test]
fn a_cert_file_that_is_a_fifo_fails_as_not_a_file() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("bundle.fifo");
    let made = std::process::Command::new("mkfifo")
        .arg(&path)
        .status()
        .expect("mkfifo runs");
    assert!(made.success(), "mkfifo failed: {made}");

    let error = route_with_cert_file(&path).expect_err("a FIFO");

    assert_eq!(
        reason(&error),
        format!("SSL_CERT_FILE ({}) is not a file", path.display())
    );
}

/// `SSL_CERT_FILE` is often a link to the system bundle: the check follows it.
#[cfg(unix)]
#[test]
fn a_cert_file_symlinked_to_a_bundle_is_read() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let target = dir.path().join("bundle.crt");
    std::fs::write(&target, TEST_ROOT).expect("the bundle is written");
    let link = dir.path().join("ca-certificates.crt");
    std::os::unix::fs::symlink(&target, &link).expect("the link is made");

    let route = route_with_cert_file(&link).expect("a route");

    assert!(
        matches!(&route.roots, RootCerts::Specific(certs) if certs.len() == 1),
        "{route:?}"
    );
}

#[test]
fn the_network_s_debug_never_shows_the_proxy_value() {
    let network = Network::read(lookup(&[(
        "HTTPS_PROXY",
        "http://user:s3cret@10.0.0.1:3128",
    )]));

    let shown = format!("{network:?}");

    assert!(shown.contains("HTTPS_PROXY"), "{shown}");
    assert!(!shown.contains("s3cret"), "{shown}");
}
