//! Problem mode hook for blocking tool use during problem explanation.
//!
//! When a user enters problem mode (by saying "I have run into a problem"),
//! all tool use is blocked until they stop and explain their problem to the user.

use crate::hooks::{HookInput, PreToolUseOutput};
use crate::session;
use crate::templates;
use std::path::Path;
use tera::Context;

/// Run the problem mode `PreToolUse` hook.
///
/// This hook checks if problem mode is active and blocks all tool use if so.
/// Problem mode is activated when the bot says "I have run into a problem"
/// and is deactivated when they successfully stop.
///
/// # Panics
///
/// Panics if embedded templates fail to render. Templates are embedded via
/// `include_str!` and verified by `test_all_embedded_templates_render`, so
/// this should only occur if a template has a bug that escaped tests.
pub fn run_problem_mode_hook(input: &HookInput, base_dir: &Path) -> PreToolUseOutput {
    // Check if problem mode is active
    if !session::is_problem_mode_active(base_dir) {
        return PreToolUseOutput::allow(None);
    }

    // Get tool name for context
    let tool_name = input.tool_name.as_deref().unwrap_or("Unknown");

    // Block all tool use in problem mode
    let mut ctx = Context::new();
    ctx.insert("tool_name", tool_name);

    let context = templates::render("messages/problem_mode_block.tera", &ctx)
        .expect("problem_mode_block.tera template should always render");

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
        assert!(json.contains("Problem Mode"));
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
