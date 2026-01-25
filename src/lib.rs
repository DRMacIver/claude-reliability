//! # `claude_reliability`
//!
//! Hooks for improving Claude Code reliability and safety.

pub mod beads_sync;
pub mod cli;
pub mod command;
pub mod config;
pub mod error;
pub mod git;
pub mod hook_logging;
pub mod hooks;
pub mod mcp;
pub mod paths;
pub mod question;
pub mod session;
pub mod storage;
pub mod subagent;
pub mod tasks;
pub mod templates;
pub mod testing;
pub mod traits;
pub mod transcript;

/// Library version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_exists() {
        assert!(!VERSION.is_empty());
    }
}
