//! `anneal`: the deterministic layer of the anneal plugin. Everything but process plumbing
//! lives in the `anneal_cli` library.

use std::io::{self, BufWriter};
use std::process::ExitCode;

fn main() -> ExitCode {
    let stdout = io::stdout();
    let stderr = io::stderr();
    anneal_cli::cli::run(
        std::env::args_os(),
        BufWriter::new(stdout.lock()),
        stderr.lock(),
    )
    .into()
}
