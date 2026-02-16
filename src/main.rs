//! CLI binary for `claude_reliability` hooks.
//!
//! This binary uses clap for argument parsing and delegates to the library.

use std::io::{self, Read};
use std::process::ExitCode;

use clap::Parser;
use claude_reliability::cli::Cli;

fn main() -> ExitCode {
    // Always enable full backtraces for panic diagnostics unless explicitly overridden.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        std::env::set_var("RUST_BACKTRACE", "full");
    }

    let cli = Cli::parse();

    let stdin = if cli.command.needs_stdin() { read_stdin() } else { String::new() };

    let output = claude_reliability::cli::run(cli.command, &stdin);

    for msg in output.stdout {
        println!("{msg}");
    }
    for msg in output.stderr {
        eprintln!("{msg}");
    }

    output.exit_code
}

fn read_stdin() -> String {
    let mut input = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut input) {
        eprintln!("Error reading stdin: {e}");
    }
    input
}
