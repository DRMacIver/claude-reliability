//! Error types for `claude_reliability`.

use std::path::PathBuf;

/// Errors that can occur in the reliability hooks.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An I/O error occurred.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A JSON parsing error occurred.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// A YAML parsing error occurred.
    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    /// A `SQLite` database error occurred.
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// A regex error occurred.
    #[error("Regex error: {0}")]
    Regex(#[from] regex::Error),

    /// A command execution failed.
    #[error("Command '{command}' failed with exit code {exit_code}: {stderr}")]
    CommandFailed {
        /// The command that was run.
        command: String,
        /// The exit code.
        exit_code: i32,
        /// The stderr output.
        stderr: String,
    },

    /// A command timed out.
    #[error("Command '{command}' timed out after {timeout_secs} seconds")]
    CommandTimeout {
        /// The command that was run.
        command: String,
        /// The timeout in seconds.
        timeout_secs: u64,
    },

    /// A file was not found.
    #[error("File not found: {0}")]
    FileNotFound(PathBuf),

    /// Invalid transcript format.
    #[error("Invalid transcript format: {0}")]
    InvalidTranscript(String),

    /// Invalid session file format.
    #[error("Invalid session file format: {0}")]
    InvalidSessionFile(String),

    /// Git is not available or not in a git repository.
    #[error("Git error: {0}")]
    Git(String),

    /// A template error occurred.
    #[error("Template error: {0}")]
    Template(String),

    /// A task-related error occurred.
    #[error("{0}")]
    Task(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// A specialized Result type for this crate.
pub type Result<T> = std::result::Result<T, Error>;
