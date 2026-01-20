//! Core traits for testability and abstraction.

use crate::error::Result;
use std::time::Duration;

/// Output from a command execution.
#[derive(Debug, Clone, Default)]
pub struct CommandOutput {
    /// The exit code of the command.
    pub exit_code: i32,
    /// The stdout output.
    pub stdout: String,
    /// The stderr output.
    pub stderr: String,
}

impl CommandOutput {
    /// Check if the command succeeded (exit code 0).
    #[must_use]
    pub const fn success(&self) -> bool {
        self.exit_code == 0
    }

    /// Get combined stdout and stderr.
    #[must_use]
    pub fn combined_output(&self) -> String {
        if self.stderr.is_empty() {
            self.stdout.clone()
        } else if self.stdout.is_empty() {
            self.stderr.clone()
        } else {
            format!("{}\n{}", self.stdout, self.stderr)
        }
    }
}

/// Trait for running shell commands.
///
/// This trait abstracts command execution for testability.
pub trait CommandRunner {
    /// Run a command with the given arguments and timeout.
    ///
    /// # Arguments
    ///
    /// * `program` - The program to run.
    /// * `args` - The arguments to pass.
    /// * `timeout` - Optional timeout duration.
    ///
    /// # Returns
    ///
    /// The command output, or an error if the command could not be started.
    ///
    /// # Errors
    ///
    /// Returns an error if the command cannot be spawned or executed.
    fn run(&self, program: &str, args: &[&str], timeout: Option<Duration>)
        -> Result<CommandOutput>;

    /// Check if a program is available in PATH.
    fn is_available(&self, program: &str) -> bool;
}

/// Decision from a sub-agent about whether to allow stopping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubAgentDecision {
    /// Allow the stop, optionally with a reason.
    AllowStop(Option<String>),
    /// Block the stop and provide an answer to continue with.
    Answer(String),
    /// Block the stop and continue without a specific response.
    Continue,
}

/// Trait for sub-agent interactions.
///
/// This trait abstracts the Claude sub-agent calls for testability.
pub trait SubAgent {
    /// Ask the sub-agent whether to allow stopping when a question is detected.
    ///
    /// # Arguments
    ///
    /// * `assistant_output` - The assistant's last output (truncated to ~2000 chars).
    /// * `user_recency_minutes` - How recently the user was active.
    ///
    /// # Returns
    ///
    /// The sub-agent's decision.
    ///
    /// # Errors
    ///
    /// Returns an error if the sub-agent call fails.
    fn decide_on_question(
        &self,
        assistant_output: &str,
        user_recency_minutes: u32,
    ) -> Result<SubAgentDecision>;

    /// Run a code review on the given diff.
    ///
    /// # Arguments
    ///
    /// * `diff` - The git diff to review.
    /// * `files` - The list of files being committed.
    /// * `review_guide` - Optional review guidelines from REVIEWGUIDE.md.
    ///
    /// # Returns
    ///
    /// A tuple of (approved, feedback).
    ///
    /// # Errors
    ///
    /// Returns an error if the sub-agent call fails.
    fn review_code(
        &self,
        diff: &str,
        files: &[String],
        review_guide: Option<&str>,
    ) -> Result<(bool, String)>;

    /// Reflect on the work done and check if it meets the user's request.
    ///
    /// # Arguments
    ///
    /// * `assistant_output` - The assistant's last output (truncated).
    /// * `git_diff` - The git diff showing changes made.
    /// * `in_jkw_mode` - Whether we're currently in just-keep-working mode.
    ///
    /// # Returns
    ///
    /// A tuple of (`work_complete`, feedback). If `work_complete` is false,
    /// the feedback describes what may be incomplete or missing.
    ///
    /// # Errors
    ///
    /// Returns an error if the sub-agent call fails.
    fn reflect_on_work(
        &self,
        assistant_output: &str,
        git_diff: &str,
        in_jkw_mode: bool,
    ) -> Result<(bool, String)>;
}
