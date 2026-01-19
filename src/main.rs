//! CLI binary for `claude_reliability` hooks.
//!
//! This binary is a thin wrapper around the library CLI module.

use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    claude_reliability::cli::run(&args)
}
