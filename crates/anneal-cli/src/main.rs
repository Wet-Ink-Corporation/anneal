//! `anneal`: the deterministic layer of the anneal plugin.
//!
//! This is the scaffold: it answers `--help` and `--version` so the plugin launcher can be
//! smoke-tested. The command tree, JSON output and typed exit codes land with the CLI core.

use std::ffi::OsString;
use std::io::{self, Write};
use std::process::ExitCode;

const USAGE: &str = "\
anneal: enforced SDLC gates for Claude Code

Usage: anneal [--help | --version]

Options:
  -h, --help     Print this help
  -V, --version  Print the version
";

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
enum Request {
    Help,
    Version,
    Unknown(OsString),
}

fn parse(mut args: impl Iterator<Item = OsString>) -> Request {
    match args.next() {
        None => Request::Help,
        Some(arg) if arg == "-h" || arg == "--help" => Request::Help,
        Some(arg) if arg == "-V" || arg == "--version" => Request::Version,
        Some(arg) => Request::Unknown(arg),
    }
}

fn run(request: &Request, out: &mut impl Write, err: &mut impl Write) -> io::Result<ExitCode> {
    match request {
        Request::Help => {
            out.write_all(USAGE.as_bytes())?;
            Ok(ExitCode::SUCCESS)
        }
        Request::Version => {
            writeln!(out, "anneal {}", env!("CARGO_PKG_VERSION"))?;
            Ok(ExitCode::SUCCESS)
        }
        Request::Unknown(arg) => {
            writeln!(err, "ERROR unknown argument: {}", arg.to_string_lossy())?;
            err.write_all(USAGE.as_bytes())?;
            Ok(ExitCode::from(2))
        }
    }
}

fn main() -> ExitCode {
    let request = parse(std::env::args_os().skip(1));
    let stdout = io::stdout();
    let stderr = io::stderr();
    // A write error means stdout or stderr is closed, so there is nowhere left to report it:
    // the failure exit code is the report.
    match run(&request, &mut stdout.lock(), &mut stderr.lock()) {
        Ok(code) => code,
        Err(_closed) => ExitCode::FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> impl Iterator<Item = OsString> {
        list.iter()
            .map(OsString::from)
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn no_arguments_asks_for_help() {
        assert_eq!(
            parse(args(&[])),
            Request::Help,
            "bare `anneal` should print help"
        );
    }

    #[test]
    fn help_and_version_flags_parse() {
        assert_eq!(parse(args(&["--help"])), Request::Help, "--help");
        assert_eq!(parse(args(&["-h"])), Request::Help, "-h");
        assert_eq!(parse(args(&["--version"])), Request::Version, "--version");
        assert_eq!(parse(args(&["-V"])), Request::Version, "-V");
    }

    #[test]
    fn unknown_argument_exits_2_with_one_error_line() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(&Request::Unknown("nope".into()), &mut out, &mut err)
            .expect("writing to a Vec cannot fail");
        assert_eq!(code, ExitCode::from(2), "usage errors exit 2");
        assert!(out.is_empty(), "nothing on stdout for a usage error");
        let err = String::from_utf8(err).expect("the message is UTF-8");
        assert!(
            err.starts_with("ERROR unknown argument: nope\n"),
            "got {err:?}"
        );
    }

    #[test]
    fn version_prints_the_package_version() {
        let mut out = Vec::new();
        let code = run(&Request::Version, &mut out, &mut Vec::new())
            .expect("writing to a Vec cannot fail");
        assert_eq!(code, ExitCode::SUCCESS, "--version succeeds");
        assert_eq!(
            String::from_utf8(out).expect("the version line is UTF-8"),
            format!("anneal {}\n", env!("CARGO_PKG_VERSION")),
            "--version prints the manifest version"
        );
    }
}
