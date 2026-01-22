//! Core traits for testability and abstraction.

use crate::error::Result;
use crate::session::SessionConfig;
use std::collections::HashSet;
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

/// Context for question decision-making.
#[derive(Debug, Clone)]
pub struct QuestionContext {
    /// The assistant's last output (truncated).
    pub assistant_output: String,
    /// How recently the user was active (in minutes).
    pub user_recency_minutes: u32,
    /// Human-readable timestamp of when the user was last active.
    pub user_last_active: Option<String>,
    /// Whether any modifying tool calls were made since the user last spoke.
    pub has_modifications_since_user: bool,
}

/// Trait for sub-agent interactions.
///
/// This trait abstracts the Claude sub-agent calls for testability.
pub trait SubAgent {
    /// Ask the sub-agent whether to allow stopping when a question is detected.
    ///
    /// # Arguments
    ///
    /// * `context` - The context for making the decision.
    ///
    /// # Returns
    ///
    /// The sub-agent's decision.
    ///
    /// # Errors
    ///
    /// Returns an error if the sub-agent call fails.
    fn decide_on_question(&self, context: &QuestionContext) -> Result<SubAgentDecision>;

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
}

/// Trait for persistent state storage.
///
/// This trait abstracts state storage operations for testability.
/// The production implementation uses `SQLite`, while tests use an in-memory mock.
pub trait StateStore {
    /// Get the current session state (JKW mode).
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails.
    fn get_session_state(&self) -> Result<Option<SessionConfig>>;

    /// Set the session state (JKW mode).
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails.
    fn set_session_state(&self, state: &SessionConfig) -> Result<()>;

    /// Clear the session state (JKW mode ended).
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails.
    fn clear_session_state(&self) -> Result<()>;

    /// Get the issue snapshot (set of issue IDs).
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails.
    fn get_issue_snapshot(&self) -> Result<HashSet<String>>;

    /// Set the issue snapshot (list of issue IDs).
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails.
    fn set_issue_snapshot(&self, issues: &[String]) -> Result<()>;

    /// Check if a boolean marker is set.
    fn has_marker(&self, name: &str) -> bool;

    /// Set a boolean marker.
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails.
    fn set_marker(&self, name: &str) -> Result<()>;

    /// Clear a boolean marker.
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails.
    fn clear_marker(&self, name: &str) -> Result<()>;
}
