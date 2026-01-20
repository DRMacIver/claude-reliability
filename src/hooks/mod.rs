//! Hook implementations for Claude Code.

mod code_review;
mod no_verify;
mod stop;
mod user_prompt_submit;

pub use code_review::{run_code_review_hook, CodeReviewConfig};
pub use no_verify::run_no_verify_hook;
pub use stop::{run_stop_hook, StopHookConfig, StopHookResult};
pub use user_prompt_submit::run_user_prompt_submit_hook;

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

/// Tool input for Bash commands.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ToolInput {
    /// The command being executed.
    #[serde(default)]
    pub command: Option<String>,
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
}
