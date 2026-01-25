//! Real command execution implementation.

use crate::error::Result;
use crate::traits::{CommandOutput, CommandRunner};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// Format a command and its arguments into a string for error messages.
pub fn format_command(program: &str, args: &[&str]) -> String {
    if args.is_empty() {
        program.to_string()
    } else {
        format!("{} {}", program, args.join(" "))
    }
}

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

/// Wait for a child process with a timeout.
///
/// Polls the child every 100ms until it exits or the timeout expires.
/// If the timeout expires, kills the child and returns an error.
fn wait_with_timeout(
    child: &mut Child,
    timeout: Duration,
    program: &str,
    args: &[&str],
) -> Result<std::process::Output> {
    use crate::error::Error;
    use std::time::Instant;

    let start = Instant::now();
    let poll_interval = Duration::from_millis(100);

    loop {
        // Use ? to propagate io::Error via the From trait (which is tested elsewhere)
        if let Some(status) = child.try_wait()? {
            // Child has exited - collect output
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            if let Some(ref mut out) = child.stdout {
                std::io::Read::read_to_end(out, &mut stdout)?;
            }
            if let Some(ref mut err) = child.stderr {
                std::io::Read::read_to_end(err, &mut stderr)?;
            }
            return Ok(std::process::Output { status, stdout, stderr });
        }
        // Still running - check timeout
        if start.elapsed() >= timeout {
            // Timeout expired - kill the child
            let _ = child.kill();
            let _ = child.wait(); // Reap the zombie

            return Err(Error::CommandTimeout {
                command: format_command(program, args),
                timeout_secs: timeout.as_secs(),
            });
        }
        std::thread::sleep(poll_interval);
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

        let mut child = spawn_with_etxtbsy_retry(|| command.spawn())?;

        // Handle timeout if specified
        let output = if let Some(timeout_duration) = timeout {
            wait_with_timeout(&mut child, timeout_duration, program, args)?
        } else {
            child.wait_with_output()?
        };

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

        Ok(CommandOutput { exit_code, stdout, stderr })
    }

    fn run_in_dir(
        &self,
        program: &str,
        args: &[&str],
        timeout: Option<Duration>,
        cwd: &std::path::Path,
    ) -> Result<CommandOutput> {
        let mut command = Command::new(program);
        command.args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).current_dir(cwd);

        let mut child = spawn_with_etxtbsy_retry(|| command.spawn())?;

        // Handle timeout if specified
        let output = if let Some(timeout_duration) = timeout {
            wait_with_timeout(&mut child, timeout_duration, program, args)?
        } else {
            child.wait_with_output()?
        };

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

    #[test]
    fn test_run_with_timeout_fast_command() {
        // A fast command should complete well before the timeout
        let runner = RealCommandRunner::new();
        let output = runner.run("echo", &["hello"], Some(Duration::from_secs(10))).unwrap();
        assert!(output.success());
        assert_eq!(output.stdout.trim(), "hello");
    }

    #[test]
    fn test_run_with_timeout_command_times_out() {
        // A slow command should be killed when timeout expires
        let runner = RealCommandRunner::new();
        let result = runner.run("sleep", &["10"], Some(Duration::from_millis(100)));

        assert!(result.is_err());
        let err = result.unwrap_err();
        let err_str = err.to_string();
        assert!(err_str.contains("timed out"), "Error should mention timeout: {err_str}");
    }

    #[test]
    fn test_format_command_empty_args() {
        // Test format_command with empty args
        let result = super::format_command("test_program", &[]);
        assert_eq!(result, "test_program");
    }

    #[test]
    fn test_format_command_with_args() {
        // Test format_command with args
        let result = super::format_command("echo", &["hello", "world"]);
        assert_eq!(result, "echo hello world");
    }

    #[test]
    fn test_run_with_timeout_empty_args_via_timeout_runner() {
        // Test that TimeoutCommandRunner correctly formats command without args
        use crate::testing::TimeoutCommandRunner;
        let runner = TimeoutCommandRunner::new(300);
        // Test with empty args
        let result = runner.run("test_program", &[], None);

        assert!(result.is_err());
        let err = result.unwrap_err();
        let err_str = err.to_string();
        assert!(err_str.contains("timed out"), "Error should mention timeout: {err_str}");
        // Error message should just be the program name without args
        assert!(
            err_str.contains("'test_program'"),
            "Error should show command without args: {err_str}"
        );
    }

    #[test]
    fn test_run_without_timeout_waits_indefinitely() {
        // Without a timeout, should wait for the command to complete
        let runner = RealCommandRunner::new();
        // Use a command that takes a bit but not too long
        let output = runner.run("sleep", &["0.1"], None).unwrap();
        assert!(output.success());
    }

    #[test]
    fn test_run_in_dir_basic() {
        use std::path::Path;

        let runner = RealCommandRunner::new();
        // Run pwd in /tmp to verify directory change
        let output = runner.run_in_dir("pwd", &[], None, Path::new("/tmp")).unwrap();
        assert!(output.success());
        // pwd output should contain /tmp (may be /private/tmp on macOS)
        assert!(
            output.stdout.contains("/tmp"),
            "Expected /tmp in output, got: {}",
            output.stdout.trim()
        );
    }

    #[test]
    fn test_run_in_dir_with_timeout() {
        use std::path::Path;

        let runner = RealCommandRunner::new();
        let output =
            runner.run_in_dir("echo", &["hello"], Some(Duration::from_secs(10)), Path::new("/tmp"));
        assert!(output.is_ok());
        assert_eq!(output.unwrap().stdout.trim(), "hello");
    }
}
