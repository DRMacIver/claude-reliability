//! Real command execution implementation.

use crate::error::Result;
use crate::traits::{CommandOutput, CommandRunner};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Real command runner that executes shell commands.
#[derive(Debug, Default, Clone)]
pub struct RealCommandRunner;

impl RealCommandRunner {
    /// Create a new command runner.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl CommandRunner for RealCommandRunner {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        timeout: Option<Duration>,
    ) -> Result<CommandOutput> {
        let mut command = Command::new(program);
        command.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());

        let child = command.spawn()?;

        // Handle timeout if specified
        // Note: For now, we don't actually implement timeout - that would require
        // spawning a separate thread or using async. We just use blocking wait.
        let _ = timeout; // Acknowledge the parameter
        let output = child.wait_with_output()?;

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

        Ok(CommandOutput { exit_code, stdout, stderr })
    }

    fn is_available(&self, program: &str) -> bool {
        Command::new("which")
            .arg(program)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_echo() {
        let runner = RealCommandRunner::new();
        let output = runner.run("echo", &["hello"], None).unwrap();
        assert!(output.success());
        assert_eq!(output.stdout.trim(), "hello");
    }

    #[test]
    fn test_run_failing_command() {
        let runner = RealCommandRunner::new();
        let output = runner.run("false", &[], None).unwrap();
        assert!(!output.success());
        assert_ne!(output.exit_code, 0);
    }

    #[test]
    fn test_is_available() {
        let runner = RealCommandRunner::new();
        assert!(runner.is_available("echo"));
        assert!(!runner.is_available("definitely_not_a_real_command_12345"));
    }

    #[test]
    fn test_combined_output() {
        let output =
            CommandOutput { exit_code: 0, stdout: "out".to_string(), stderr: "err".to_string() };
        assert_eq!(output.combined_output(), "out\nerr");

        let stdout_only =
            CommandOutput { exit_code: 0, stdout: "out".to_string(), stderr: String::new() };
        assert_eq!(stdout_only.combined_output(), "out");

        let stderr_only =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: "err".to_string() };
        assert_eq!(stderr_only.combined_output(), "err");
    }
}
