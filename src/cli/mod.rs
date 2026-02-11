//! Hierarchical CLI for claude-reliability.
//!
//! This module provides the command-line interface with two-level commands
//! for managing work items, how-to guides, questions, and other operations.

mod howto;
mod question;
mod run;
mod work;

#[cfg(test)]
mod tests;

pub use howto::HowToCommand;
pub use question::QuestionCommand;
pub use run::{run, CliOutput};
pub use work::WorkCommand;

use clap::{Parser, Subcommand};

/// Claude reliability CLI - work tracking and session management.
///
/// For detailed help on any command group, use:
///   claude-reliability <command> --help
///
/// Skills with more information:
///   - task-management: Managing work items and dependencies
///   - deciding-what-to-work-on: Choosing which work to tackle
///   - documentation: Creating how-to guides
#[derive(Parser, Debug)]
#[command(name = "claude-reliability")]
#[command(version, about, long_about = None)]
#[command(propagate_version = true)]
pub struct Cli {
    /// The command to execute
    #[command(subcommand)]
    pub command: Command,
}

/// Top-level commands.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Work item management - create, update, list, and track work items.
    ///
    /// Work items are the core unit of task tracking. Use these commands to:
    /// - Create new work items with priorities
    /// - Track dependencies between items
    /// - Mark items as in-progress or complete
    /// - Find the next item to work on
    ///
    /// See skill: task-management
    #[command(subcommand)]
    Work(WorkCommand),

    /// How-to guide management - create and manage reusable procedures.
    ///
    /// How-to guides capture reusable procedures that can be linked to
    /// work items. When you retrieve a work item, linked how-tos appear
    /// with their full instructions.
    ///
    /// See skill: documentation
    #[command(subcommand)]
    Howto(HowToCommand),

    /// Question management - create questions requiring user input.
    ///
    /// Questions can block work items until answered. Use these when you
    /// need user input or clarification before proceeding with work.
    ///
    /// See skill: requirements
    #[command(subcommand)]
    Question(QuestionCommand),

    /// Get the audit log for work item changes.
    ///
    /// Shows a history of changes to work items, including creation,
    /// updates, status changes, and dependency modifications.
    #[command(name = "audit-log")]
    AuditLog {
        /// Filter by work item ID
        #[arg(long)]
        work_id: Option<String>,

        /// Maximum number of entries to return
        #[arg(long)]
        limit: Option<usize>,
    },

    /// Request an emergency stop (validated by sub-agent).
    ///
    /// Use this only for genuine blockers that require user intervention:
    /// - Missing credentials or environment issues
    /// - Unclear requirements that prevent ANY progress
    ///
    /// Do NOT use for "too much work" - that's normal. Just work through
    /// tasks one at a time using priority order.
    #[command(name = "emergency-stop")]
    EmergencyStop {
        /// Explanation of why you need to stop
        explanation: String,
    },

    // === Utility Commands ===
    /// Show version information.
    Version,

    /// Ensure config file exists (create with defaults if not).
    #[command(name = "ensure-config")]
    EnsureConfig,

    /// Ensure .gitignore has required entries.
    #[command(name = "ensure-gitignore")]
    EnsureGitignore,

    /// Print session intro message.
    Intro,

    // === Hook Commands (receive JSON from stdin) ===
    /// Run the stop hook (stdin: JSON hook input).
    ///
    /// This is called by the plugin system when Claude attempts to stop.
    /// Not intended for direct use.
    #[command(hide = true)]
    Stop,

    /// Run the user prompt submit hook (stdin: JSON hook input).
    ///
    /// This is called by the plugin system when user submits a prompt.
    /// Not intended for direct use.
    #[command(name = "user-prompt-submit", hide = true)]
    UserPromptSubmit,

    /// Run the pre-tool-use hook (stdin: JSON hook input).
    ///
    /// This is called by the plugin system before tool execution.
    /// Not intended for direct use.
    #[command(name = "pre-tool-use", hide = true)]
    PreToolUse,

    /// Run the post-tool-use hook (stdin: JSON hook input).
    ///
    /// This is called by the plugin system after tool execution.
    /// Not intended for direct use.
    #[command(name = "post-tool-use", hide = true)]
    PostToolUse,
}

impl Command {
    /// Returns true if this command requires stdin input.
    #[must_use]
    pub const fn needs_stdin(&self) -> bool {
        matches!(self, Self::Stop | Self::PreToolUse | Self::PostToolUse | Self::UserPromptSubmit)
    }

    /// Returns true if this is a hook command (invoked by the plugin system).
    #[must_use]
    pub const fn is_hook(&self) -> bool {
        matches!(self, Self::Stop | Self::PreToolUse | Self::PostToolUse | Self::UserPromptSubmit)
    }

    /// Returns the hook type name for logging, or None for non-hook commands.
    #[must_use]
    pub const fn hook_type(&self) -> Option<&'static str> {
        match self {
            Self::Stop => Some("stop"),
            Self::UserPromptSubmit => Some("user-prompt-submit"),
            Self::PreToolUse => Some("pre-tool-use"),
            Self::PostToolUse => Some("post-tool-use"),
            _ => None,
        }
    }
}
