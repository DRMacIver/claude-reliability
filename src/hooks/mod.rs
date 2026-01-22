//! Hook implementations for Claude Code.

mod code_review;
mod jkw_setup;
mod no_verify;
mod plan_tasks;
mod post_tool_use;
mod pre_tool_use;
mod problem_mode;
mod protect_config;
mod require_task;
mod stop;
mod user_prompt_submit;
mod validation;

pub use code_review::{run_code_review_hook, CodeReviewConfig};
pub use jkw_setup::run_jkw_setup_hook;
pub use no_verify::run_no_verify_hook;
pub use post_tool_use::{run_post_tool_use, PostToolUseInput};
pub use pre_tool_use::run_pre_tool_use;
pub use problem_mode::run_problem_mode_hook;
pub use protect_config::run_protect_config_hook;
pub use require_task::run_require_task_hook;
pub use stop::{run_stop_hook, StopHookConfig, StopHookResult};
pub use user_prompt_submit::run_user_prompt_submit_hook;
pub use validation::run_validation_hook;

use crate::error::Result;
use serde::{Deserialize, Serialize};

/// Input provided to hooks by Claude Code.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct HookInput {
    /// Path to the transcript file.
    #[serde(default)]
    pub transcript_path: Option<String>,
    /// The tool being called (for `PreToolUse` hooks).
    #[serde(default)]
    pub tool_name: Option<String>,
    /// The tool input (for `PreToolUse` hooks).
    #[serde(default)]
    pub tool_input: Option<ToolInput>,
}

/// Tool input for various tool types.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ToolInput {
    /// The command being executed (for Bash tool).
    #[serde(default)]
    pub command: Option<String>,
    /// The skill being invoked (for Skill tool).
    #[serde(default)]
    pub skill: Option<String>,
    /// The file path being written/edited (for Write/Edit tools).
    #[serde(default)]
    pub file_path: Option<String>,
}

/// Output from a `PreToolUse` hook.
#[derive(Debug, Clone, Serialize)]
pub struct PreToolUseOutput {
    /// Hook-specific output.
    #[serde(rename = "hookSpecificOutput")]
    pub hook_specific_output: HookSpecificOutput,
}

/// Hook-specific output for `PreToolUse`.
#[derive(Debug, Clone, Serialize)]
pub struct HookSpecificOutput {
    /// The hook event name.
    #[serde(rename = "hookEventName")]
    pub hook_event_name: String,
    /// The permission decision.
    #[serde(rename = "permissionDecision")]
    pub permission_decision: String,
    /// Additional context to provide to the agent.
    #[serde(rename = "additionalContext", skip_serializing_if = "Option::is_none")]
    pub additional_context: Option<String>,
}

impl PreToolUseOutput {
    /// Create an "allow" response.
    pub fn allow(context: Option<String>) -> Self {
        Self {
            hook_specific_output: HookSpecificOutput {
                hook_event_name: "PreToolUse".to_string(),
                permission_decision: "allow".to_string(),
                additional_context: context,
            },
        }
    }

    /// Create a "block" response.
    pub fn block(context: Option<String>) -> Self {
        Self {
            hook_specific_output: HookSpecificOutput {
                hook_event_name: "PreToolUse".to_string(),
                permission_decision: "block".to_string(),
                additional_context: context,
            },
        }
    }

    /// Check if this is a block decision.
    #[must_use]
    pub fn is_block(&self) -> bool {
        self.hook_specific_output.permission_decision == "block"
    }
}

/// Parse hook input from stdin.
///
/// # Errors
///
/// Returns an error if the input cannot be parsed as JSON.
pub fn parse_hook_input(input: &str) -> Result<HookInput> {
    if input.trim().is_empty() {
        return Ok(HookInput::default());
    }
    let parsed: HookInput = serde_json::from_str(input)?;
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hook_input_empty() {
        let input = parse_hook_input("").unwrap();
        assert!(input.transcript_path.is_none());
    }

    #[test]
    fn test_parse_hook_input_with_transcript() {
        let input = parse_hook_input(r#"{"transcript_path": "/tmp/transcript.jsonl"}"#).unwrap();
        assert_eq!(input.transcript_path, Some("/tmp/transcript.jsonl".to_string()));
    }

    #[test]
    fn test_parse_hook_input_with_tool() {
        let input = parse_hook_input(
            r#"{"tool_name": "Bash", "tool_input": {"command": "git commit -m 'test'"}}"#,
        )
        .unwrap();
        assert_eq!(input.tool_name, Some("Bash".to_string()));
        assert_eq!(input.tool_input.unwrap().command, Some("git commit -m 'test'".to_string()));
    }

    #[test]
    fn test_pre_tool_use_output_allow() {
        let output = PreToolUseOutput::allow(Some("Feedback".to_string()));
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("allow"));
        assert!(json.contains("Feedback"));
    }

    #[test]
    fn test_pre_tool_use_output_block() {
        let output = PreToolUseOutput::block(None);
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("block"));
        assert!(!json.contains("additionalContext"));
    }

    #[test]
    fn test_parse_hook_input_with_skill() {
        let input = parse_hook_input(
            r#"{"tool_name": "Skill", "tool_input": {"skill": "just-keep-working"}}"#,
        )
        .unwrap();
        assert_eq!(input.tool_name, Some("Skill".to_string()));
        let tool_input = input.tool_input.unwrap();
        assert_eq!(tool_input.skill, Some("just-keep-working".to_string()));
    }

    #[test]
    fn test_parse_hook_input_with_file_path() {
        let input = parse_hook_input(
            r#"{"tool_name": "Write", "tool_input": {"file_path": "src/main.rs"}}"#,
        )
        .unwrap();
        assert_eq!(input.tool_name, Some("Write".to_string()));
        let tool_input = input.tool_input.unwrap();
        assert_eq!(tool_input.file_path, Some("src/main.rs".to_string()));
    }
}
