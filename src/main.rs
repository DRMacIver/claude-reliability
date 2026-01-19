//! CLI binary for `claude_reliability` hooks.
//!
//! This binary provides a thin wrapper around the hook library functions.
//! It can be invoked as:
//! - `claude-reliability stop` - Run the stop hook
//! - `claude-reliability pre-tool-use no-verify` - Run the no-verify check
//! - `claude-reliability pre-tool-use code-review` - Run the code review hook

use claude_reliability::{
    command::RealCommandRunner,
    hooks::{
        parse_hook_input, run_code_review_hook, run_no_verify_hook, run_stop_hook,
        CodeReviewConfig, StopHookConfig,
    },
    subagent::RealSubAgent,
};
use std::env;
use std::io::{self, Read};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} <command> [subcommand]", args[0]);
        eprintln!();
        eprintln!("Commands:");
        eprintln!("  stop                    Run the stop hook");
        eprintln!("  pre-tool-use no-verify  Check for --no-verify usage");
        eprintln!("  pre-tool-use code-review Run code review on commits");
        eprintln!("  version                 Show version information");
        return ExitCode::from(1);
    }

    match args[1].as_str() {
        "version" | "--version" | "-v" => {
            println!("claude-reliability v{}", claude_reliability::VERSION);
            ExitCode::SUCCESS
        }
        "stop" => run_stop_hook_cli(),
        "pre-tool-use" => {
            if args.len() < 3 {
                eprintln!("Usage: {} pre-tool-use <no-verify|code-review>", args[0]);
                return ExitCode::from(1);
            }
            match args[2].as_str() {
                "no-verify" => run_no_verify_hook_cli(),
                "code-review" => run_code_review_hook_cli(),
                other => {
                    eprintln!("Unknown pre-tool-use subcommand: {other}");
                    ExitCode::from(1)
                }
            }
        }
        other => {
            eprintln!("Unknown command: {other}");
            ExitCode::from(1)
        }
    }
}

/// Convert i32 exit code to `ExitCode`, clamping to valid range.
fn exit_code_from_i32(code: i32) -> ExitCode {
    // Exit codes are typically 0-255, with 0 being success
    // Clamp to u8 range to handle negative values and values > 255
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let code_u8 = if code < 0 {
        1u8 // Treat negative as error
    } else if code > 255 {
        255u8 // Clamp to max
    } else {
        code as u8
    };
    ExitCode::from(code_u8)
}

/// Read hook input from stdin.
fn read_stdin() -> String {
    let mut input = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut input) {
        eprintln!("Error reading stdin: {e}");
    }
    input
}

/// Run the stop hook.
fn run_stop_hook_cli() -> ExitCode {
    let stdin = read_stdin();
    let input = match parse_hook_input(&stdin) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("Error parsing hook input: {e}");
            return ExitCode::from(1);
        }
    };

    let runner = RealCommandRunner::new();
    let sub_agent = RealSubAgent::new(&runner);

    // Build config from environment
    let config = StopHookConfig {
        quality_check_enabled: env::var("QUALITY_CHECK_ENABLED").is_ok(),
        quality_check_command: env::var("QUALITY_CHECK_COMMAND").ok(),
        require_push: env::var("REQUIRE_PUSH").is_ok(),
        repo_critique_mode: env::var("REPO_CRITIQUE_MODE").is_ok(),
    };

    match run_stop_hook(&input, &config, &runner, &sub_agent) {
        Ok(result) => {
            // Output messages to stderr
            for msg in &result.messages {
                eprintln!("{msg}");
            }
            exit_code_from_i32(result.exit_code)
        }
        Err(e) => {
            eprintln!("Error running stop hook: {e}");
            ExitCode::from(1)
        }
    }
}

/// Run the no-verify hook.
fn run_no_verify_hook_cli() -> ExitCode {
    let stdin = read_stdin();
    let input = match parse_hook_input(&stdin) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("Error parsing hook input: {e}");
            return ExitCode::from(1);
        }
    };

    match run_no_verify_hook(&input) {
        Ok(exit_code) => exit_code_from_i32(exit_code),
        Err(e) => {
            eprintln!("Error running no-verify hook: {e}");
            ExitCode::from(1)
        }
    }
}

/// Run the code review hook.
fn run_code_review_hook_cli() -> ExitCode {
    let stdin = read_stdin();
    let input = match parse_hook_input(&stdin) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("Error parsing hook input: {e}");
            return ExitCode::from(1);
        }
    };

    let runner = RealCommandRunner::new();
    let sub_agent = RealSubAgent::new(&runner);

    // Build config from environment
    let config = CodeReviewConfig { skip_review: env::var("SKIP_CODE_REVIEW").is_ok() };

    match run_code_review_hook(&input, &config, &runner, &sub_agent) {
        Ok(exit_code) => exit_code_from_i32(exit_code),
        Err(e) => {
            eprintln!("Error running code review hook: {e}");
            ExitCode::from(1)
        }
    }
}
