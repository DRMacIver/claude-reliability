//! Problem mode hook for blocking tool use during problem explanation.
//!
//! When a user enters problem mode (by saying "I have run into a problem"),
//! all tool use is blocked until they stop and explain their problem to the user.

use crate::hooks::{HookInput, PreToolUseOutput};
use crate::session;
use std::path::Path;

/// Run the problem mode `PreToolUse` hook.
///
/// This hook checks if problem mode is active and blocks all tool use if so.
/// Problem mode is activated when the bot says "I have run into a problem"
/// and is deactivated when they successfully stop.
pub fn run_problem_mode_hook(input: &HookInput, base_dir: &Path) -> PreToolUseOutput {
    // Check if problem mode is active
    if !session::is_problem_mode_active(base_dir) {
        return PreToolUseOutput::allow(None);
    }

    // Get tool name for context
    let tool_name = input.tool_name.as_deref().unwrap_or("Unknown");

    // Block all tool use in problem mode
    let context = format!(
        r"# Tool Use Blocked - Problem Mode Active

You indicated you've hit a problem you can't solve. **All tools are currently blocked.**

You attempted to use: `{tool_name}`

## What you must do:

1. **Stop trying to use tools** - They will all be blocked until you explain your problem
2. **Explain the problem clearly to the user:**
   - What exactly went wrong?
   - What did you try?
   - What specific help do you need?
3. **Then stop** - Your next stop will be allowed unconditionally

Once you've explained and stopped, the user will be able to respond and help you."
    );

    PreToolUseOutput::block(Some(context))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_problem_mode_not_active_allows_tools() {
        let dir = TempDir::new().unwrap();
        let input = HookInput { tool_name: Some("Bash".to_string()), ..Default::default() };

        let output = run_problem_mode_hook(&input, dir.path());
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("allow"));
    }

    #[test]
    fn test_problem_mode_active_blocks_tools() {
        let dir = TempDir::new().unwrap();
        session::enter_problem_mode(dir.path()).unwrap();

        let input = HookInput { tool_name: Some("Bash".to_string()), ..Default::default() };

        let output = run_problem_mode_hook(&input, dir.path());
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("block"));
        assert!(json.contains("Problem Mode Active"));
    }

    #[test]
    fn test_problem_mode_blocks_all_tools() {
        let dir = TempDir::new().unwrap();
        session::enter_problem_mode(dir.path()).unwrap();

        for tool in &["Bash", "Read", "Write", "Edit", "Grep", "Glob"] {
            let input = HookInput { tool_name: Some((*tool).to_string()), ..Default::default() };

            let output = run_problem_mode_hook(&input, dir.path());
            let json = serde_json::to_string(&output).unwrap();
            assert!(json.contains("block"), "Tool {tool} should be blocked");
        }
    }

    #[test]
    fn test_problem_mode_shows_tool_name_in_message() {
        let dir = TempDir::new().unwrap();
        session::enter_problem_mode(dir.path()).unwrap();

        let input = HookInput { tool_name: Some("WebSearch".to_string()), ..Default::default() };

        let output = run_problem_mode_hook(&input, dir.path());
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("WebSearch"));
    }
}
