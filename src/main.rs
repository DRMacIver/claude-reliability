//! CLI binary for `claude_reliability` hooks.
//!
//! This binary is a thin wrapper that reads stdin and delegates to the library.

use std::env;
use std::io::{self, Read};
use std::process::ExitCode;

use claude_reliability::cli::{parse_args, ParseResult};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();

    // Only read stdin for commands that need it (avoids blocking on terminal)
    let stdin = match parse_args(&args) {
        ParseResult::Command(cmd) if cmd.needs_stdin() => read_stdin(),
        _ => String::new(),
    };

    let (exit_code, messages) = claude_reliability::cli::run(&args, &stdin);

    for msg in messages {
        eprintln!("{msg}");
    }

    exit_code
}

fn read_stdin() -> String {
    let mut input = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut input) {
        eprintln!("Error reading stdin: {e}");
    }
    input
}
