//! Shared helpers: scratch git repositories, fixture copies, content hashes and an isolated
//! `anneal` command.

use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use sha2::{Digest, Sha256};

/// Runs `git <args>` in `dir`, isolated from the caller's repository and identity, and panics
/// with git's stderr if it fails.
pub(crate) fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=anneal test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// `anneal` run in `cwd`, isolated from the caller's repository, ledger overrides, Clef
/// configuration and anneal home: the plugin's `CLAUDE_PLUGIN_OPTION_*` Clef variables and
/// `CLAUDE_PLUGIN_ROOT` (a hook running the tests may set it to another plugin) are removed, and `ANNEAL_HOME` is `.anneal-home` under `cwd` when `cwd` is a temporary
/// directory (else under the system's temporary directory), so no test writes to `~/.anneal`.
/// A test that needs either sets it again. `ANNEAL_CLEF_URL` is `http://127.0.0.1:0`, which
/// every platform refuses at once, so a test that configures an account finds Clef unreachable
/// and never reaches the network; [`with_fake_clef`] gives a URL that answers.
pub(crate) fn anneal_in(cwd: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_anneal"));
    let temp = std::env::temp_dir();
    let home = if cwd.starts_with(&temp) {
        cwd.join(".anneal-home")
    } else {
        temp.join(format!("anneal-it-home-{}", std::process::id()))
    };
    command
        .current_dir(cwd)
        .env("ANNEAL_HOME", home)
        .env("ANNEAL_CLEF_URL", "http://127.0.0.1:0")
        .env_remove("ANNEAL_STORE")
        .env_remove("CLAUDE_PLUGIN_OPTION_CF_ACCOUNT_ID")
        .env_remove("CLAUDE_PLUGIN_OPTION_CF_API_TOKEN")
        .env_remove("CLAUDE_PLUGIN_OPTION_CLEF_MODE")
        .env_remove("CLAUDE_PLUGIN_ROOT")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE");
    command
}

/// What a fake Workers AI read of the one request it served.
#[derive(Debug, Default)]
pub(crate) struct Served {
    /// The request line and headers, `\r\n`-separated.
    pub(crate) head: String,
    /// The request body.
    pub(crate) body: Vec<u8>,
}

impl Served {
    /// The value of header `name` (case-insensitive), if sent.
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name).then(|| value.trim())
        })
    }
}

/// Runs `client` with the base URL of a fake Workers AI on a loopback port, for
/// `ANNEAL_CLEF_URL`; the fake answers one request with `200 OK` and `body`. Returns what
/// `client` returned and the request the fake read (empty when no request came).
pub(crate) fn with_fake_clef<T>(body: &str, client: impl FnOnce(&str) -> T) -> (T, Served) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let addr = listener.local_addr().expect("its address");
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    std::thread::scope(|scope| {
        let server = std::thread::Builder::new()
            .name("fake-clef".to_owned())
            .spawn_scoped(scope, || serve_one(&listener, response.as_bytes()))
            .expect("a server thread");
        let result = client(&format!("http://{addr}/accounts"));
        // Unblocks `accept` if the binary never connected; otherwise it waits in the backlog.
        TcpStream::connect(addr).expect("a dummy connection");
        (result, server.join().expect("the fake Clef"))
    })
}

/// Accepts one connection on `listener`, reads its request and writes `response`.
fn serve_one(listener: &TcpListener, response: &[u8]) -> Served {
    let (stream, _) = listener.accept().expect("a connection");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("a read timeout");
    let mut reader = BufReader::new(stream.try_clone().expect("a second handle"));
    let mut served = Served::default();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).expect("a header line") == 0 || line == "\r\n" {
            break;
        }
        served.head.push_str(&line);
    }
    let length = served
        .header("content-length")
        .map_or(0, |value| value.parse().expect("a numeric content-length"));
    served.body = vec![0; length];
    reader
        .read_exact(&mut served.body)
        .expect("the declared body");
    let mut stream = stream;
    // A dummy connection has closed by now; only the binary's request needs the answer.
    let wrote: io::Result<()> = stream.write_all(response);
    drop(wrote);
    served
}

/// A repository at `<root>/main` with one commit, and a linked worktree of it at
/// `<root>/linked`.
pub(crate) fn repo_with_linked_worktree(root: &Path) {
    let main = root.join("main");
    std::fs::create_dir(&main).expect("create main");
    git(&main, &["init", "-q"]);
    git(&main, &["commit", "-q", "--allow-empty", "-m", "init"]);
    git(
        &main,
        &["worktree", "add", "-q", "-b", "linked", "../linked"],
    );
}

/// `sha256:` and the lowercase hex digest of `bytes`, as the ledger records content hashes.
pub(crate) fn sha256_hex(bytes: &[u8]) -> Result<String, std::fmt::Error> {
    let mut hex = String::from("sha256:");
    for byte in Sha256::digest(bytes) {
        write!(hex, "{byte:02x}")?;
    }
    Ok(hex)
}

/// Copies the directory tree at `from` to `to`, creating `to`.
pub(crate) fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}
