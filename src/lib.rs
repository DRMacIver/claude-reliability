//! `claude_reliability` library.
//!
//! This library provides reliability hooks for Claude Code, including:
//! - Stop hook for autonomous mode and code quality checks
//! - Code review hook for pre-commit reviews
//! - No-verify check hook to prevent bypassing git hooks

pub mod analysis;
pub mod beads;
pub mod command;
pub mod error;
pub mod git;
pub mod hooks;
pub mod question;
pub mod session;
pub mod subagent;
#[cfg(test)]
pub mod testing;
#[cfg(not(test))]
mod testing;
pub mod traits;
pub mod transcript;

// Re-exports for convenience
pub use command::RealCommandRunner;
pub use error::{Error, Result};
pub use subagent::RealSubAgent;
pub use traits::{CommandOutput, CommandRunner, SubAgent, SubAgentDecision};

/// Library version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
