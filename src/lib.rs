//! # `claude_reliability`
//!
//! Hooks and analysis tools for improving Claude Code reliability and safety.

pub mod analysis;
pub mod beads;
pub mod cli;
pub mod command;
pub mod config;
pub mod error;
pub mod git;
pub mod hooks;
pub mod question;
pub mod reflection;
pub mod session;
pub mod subagent;
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
