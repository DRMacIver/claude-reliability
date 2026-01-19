//! Real command execution implementation.

use crate::error::Result;
use crate::traits::{CommandOutput, CommandRunner};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// ETXTBSY error code (errno 26 on Linux).
/// This error occurs when trying to execute a file that is currently being written.
const ETXTBSY: i32 = 26;

/// Spawn a command with retry logic for ETXTBSY errors.
///
/// ETXTBSY ("Text file busy") can occur on overlay filesystems (like Docker)
/// when executing a script that was just created. The file may still be held
/// open by the filesystem layer. A brief retry usually succeeds.
///
/// # Arguments
/// * `spawn_fn` - A function that attempts to spawn the process
///
/// # Returns
/// The spawned child process, or an error if spawning fails for non-ETXTBSY reasons.
fn spawn_with_etxtbsy_retry<F>(mut spawn_fn: F) -> std::io::Result<Child>
where
    F: FnMut() -> std::io::Result<Child>,
{
    loop {
        match spawn_fn() {
            Ok(child) => return Ok(child),
            Err(e) if e.raw_os_error() == Some(ETXTBSY) => {
                // ETXTBSY - wait briefly and retry
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(e) => return Err(e),
        }
    }
}

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

        let child = spawn_with_etxtbsy_retry(|| command.spawn())?;

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

    #[test]
    fn test_run_nonexistent_command() {
        let runner = RealCommandRunner::new();
        // Running a command that doesn't exist should return an error
        let result = runner.run("definitely_not_a_real_command_12345", &[], None);
        assert!(result.is_err());
    }

    #[test]
    fn test_spawn_with_etxtbsy_retry_immediate_success() {
        // Test that immediate success works
        let mut call_count = 0;
        let mut command = Command::new("true");
        command.stdout(Stdio::piped()).stderr(Stdio::piped());

        let result = spawn_with_etxtbsy_retry(|| {
            call_count += 1;
            command.spawn()
        });

        assert!(result.is_ok());
        assert_eq!(call_count, 1);
    }

    #[test]
    fn test_spawn_with_etxtbsy_retry_retries_on_etxtbsy() {
        // Test that ETXTBSY errors trigger retries
        let mut call_count = 0;
        let mut command = Command::new("true");
        command.stdout(Stdio::piped()).stderr(Stdio::piped());

        let result = spawn_with_etxtbsy_retry(|| {
            call_count += 1;
            if call_count < 3 {
                // Simulate ETXTBSY error
                Err(std::io::Error::from_raw_os_error(ETXTBSY))
            } else {
                // Succeed on third attempt
                command.spawn()
            }
        });

        assert!(result.is_ok());
        assert_eq!(call_count, 3); // Should have retried twice
    }

    #[test]
    fn test_spawn_with_etxtbsy_retry_propagates_other_errors() {
        // Test that non-ETXTBSY errors are propagated immediately
        let mut call_count = 0;

        let result = spawn_with_etxtbsy_retry(|| {
            call_count += 1;
            // Return ENOENT (No such file or directory) - should not retry
            Err(std::io::Error::from_raw_os_error(2))
        });

        assert!(result.is_err());
        assert_eq!(call_count, 1); // Should not have retried
        assert_eq!(result.unwrap_err().raw_os_error(), Some(2));
    }

    #[test]
    fn test_etxtbsy_constant() {
        // Verify the ETXTBSY constant is correct for Linux
        assert_eq!(ETXTBSY, 26);
    }
}
