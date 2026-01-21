//! Config protection hook for preventing modifications to reliability config.
//!
//! This hook blocks Write, Edit, and delete operations targeting the
//! reliability config file to prevent accidental modifications.
//!
//! It also protects JKW session files from deletion via Bash rm commands,
//! since the stop hook handles cleanup automatically when JKW mode ends.

use crate::hooks::{HookInput, PreToolUseOutput};
use crate::templates;
use tera::Context;

/// The protected config file path (relative to project root).
const PROTECTED_CONFIG: &str = ".claude/reliability-config.yaml";

/// JKW session files that should not be deleted manually (stop hook handles cleanup).
const JKW_SESSION_FILES: &[&str] =
    &[".claude/jkw-session.local.md", ".claude/jkw-state.local.yaml"];

/// Check if a path matches the protected config file.
fn is_protected_path(path: &str) -> bool {
    // Normalize the path by removing leading ./ or /
    let normalized = path.trim_start_matches("./").trim_start_matches('/');

    // Check for exact match or if it ends with the config path
    normalized == PROTECTED_CONFIG || normalized.ends_with(PROTECTED_CONFIG)
}

/// Check if a bash command might delete the config file.
fn is_config_delete_command(command: &str) -> bool {
    // Check for rm commands targeting the config
    if command.contains("rm ") || command.contains("rm\t") {
        return command.contains(PROTECTED_CONFIG)
            || command.contains("reliability-config.yaml")
            || command.contains("reliability-config");
    }

    // Check for other destructive patterns
    if command.contains("> ") && command.contains(PROTECTED_CONFIG) {
        return true; // Redirect overwrite
    }

    false
}

/// Check if a bash command attempts to delete JKW session files.
fn is_jkw_session_delete_command(command: &str) -> bool {
    // Only check rm commands
    if !command.contains("rm ") && !command.contains("rm\t") {
        return false;
    }

    for session_file in JKW_SESSION_FILES {
        if command.contains(session_file) {
            return true;
        }
        // Also check for just the filename
        if let Some(filename) = session_file.rsplit('/').next() {
            if command.contains(filename) {
                return true;
            }
        }
    }

    false
}

/// Run the config protection `PreToolUse` hook.
///
/// This hook blocks Write, Edit, and delete operations on the reliability config.
///
/// # Panics
///
/// Panics if embedded templates fail to render. Templates are embedded via
/// `include_str!` and verified by `test_all_embedded_templates_render`, so
/// this should only occur if a template has a bug that escaped tests.
pub fn run_protect_config_hook(input: &HookInput) -> PreToolUseOutput {
    let tool_name = input.tool_name.as_deref().unwrap_or("");

    match tool_name {
        "Write" | "Edit" => {
            if let Some(ref tool_input) = input.tool_input {
                if let Some(ref file_path) = tool_input.file_path {
                    if is_protected_path(file_path) {
                        let mut ctx = Context::new();
                        ctx.insert("config_path", PROTECTED_CONFIG);

                        let message = templates::render("messages/protect_config_write.tera", &ctx)
                            .expect("protect_config_write.tera template should always render");

                        return PreToolUseOutput::block(Some(message));
                    }
                }
            }
        }
        "Bash" => {
            if let Some(ref tool_input) = input.tool_input {
                if let Some(ref command) = tool_input.command {
                    if is_config_delete_command(command) {
                        let mut ctx = Context::new();
                        ctx.insert("config_path", PROTECTED_CONFIG);

                        let message =
                            templates::render("messages/protect_config_delete.tera", &ctx)
                                .expect("protect_config_delete.tera template should always render");

                        return PreToolUseOutput::block(Some(message));
                    }

                    if is_jkw_session_delete_command(command) {
                        let message =
                            templates::render("messages/protect_jkw_session.tera", &Context::new())
                                .expect("protect_jkw_session.tera template should always render");

                        return PreToolUseOutput::block(Some(message));
                    }
                }
            }
        }
        _ => {}
    }

    PreToolUseOutput::allow(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::ToolInput;

    #[test]
    fn test_is_protected_path_exact() {
        assert!(is_protected_path(".claude/reliability-config.yaml"));
    }

    #[test]
    fn test_is_protected_path_with_prefix() {
        assert!(is_protected_path("./.claude/reliability-config.yaml"));
        assert!(is_protected_path("/.claude/reliability-config.yaml"));
    }

    #[test]
    fn test_is_protected_path_absolute() {
        assert!(is_protected_path("/workspaces/project/.claude/reliability-config.yaml"));
    }

    #[test]
    fn test_is_protected_path_other_files() {
        assert!(!is_protected_path(".claude/settings.json"));
        assert!(!is_protected_path("src/main.rs"));
        assert!(!is_protected_path("reliability-config.yaml")); // Not in .claude/
    }

    #[test]
    fn test_is_config_delete_command_rm() {
        assert!(is_config_delete_command("rm .claude/reliability-config.yaml"));
        assert!(is_config_delete_command("rm -f .claude/reliability-config.yaml"));
        assert!(is_config_delete_command("rm -rf .claude/reliability-config.yaml"));
    }

    #[test]
    fn test_is_config_delete_command_not_config() {
        assert!(!is_config_delete_command("rm other-file.txt"));
        assert!(!is_config_delete_command("rm -rf target/"));
    }

    #[test]
    fn test_is_config_delete_command_redirect() {
        assert!(is_config_delete_command("echo '' > .claude/reliability-config.yaml"));
    }

    #[test]
    fn test_is_jkw_session_delete_command_session_file() {
        assert!(is_jkw_session_delete_command("rm .claude/jkw-session.local.md"));
        assert!(is_jkw_session_delete_command("rm -f .claude/jkw-session.local.md"));
    }

    #[test]
    fn test_is_jkw_session_delete_command_state_file() {
        assert!(is_jkw_session_delete_command("rm .claude/jkw-state.local.yaml"));
        assert!(is_jkw_session_delete_command("rm -rf .claude/jkw-state.local.yaml"));
    }

    #[test]
    fn test_is_jkw_session_delete_command_by_filename() {
        // Also matches just the filename
        assert!(is_jkw_session_delete_command("rm jkw-session.local.md"));
        assert!(is_jkw_session_delete_command("rm jkw-state.local.yaml"));
    }

    #[test]
    fn test_is_jkw_session_delete_command_other_files() {
        assert!(!is_jkw_session_delete_command("rm other-file.txt"));
        assert!(!is_jkw_session_delete_command("rm .claude/settings.json"));
        assert!(!is_jkw_session_delete_command("cat jkw-session.local.md")); // Not rm
    }

    #[test]
    fn test_write_to_config_blocked() {
        let input = HookInput {
            tool_name: Some("Write".to_string()),
            tool_input: Some(ToolInput {
                file_path: Some(".claude/reliability-config.yaml".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let output = run_protect_config_hook(&input);
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("block"));
        assert!(json.contains("Protected File"));
    }

    #[test]
    fn test_edit_to_config_blocked() {
        let input = HookInput {
            tool_name: Some("Edit".to_string()),
            tool_input: Some(ToolInput {
                file_path: Some(".claude/reliability-config.yaml".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let output = run_protect_config_hook(&input);
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("block"));
    }

    #[test]
    fn test_delete_config_blocked() {
        let input = HookInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(ToolInput {
                command: Some("rm .claude/reliability-config.yaml".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let output = run_protect_config_hook(&input);
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("block"));
        assert!(json.contains("Deletion Blocked"));
    }

    #[test]
    fn test_write_to_other_file_allowed() {
        let input = HookInput {
            tool_name: Some("Write".to_string()),
            tool_input: Some(ToolInput {
                file_path: Some("src/main.rs".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let output = run_protect_config_hook(&input);
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("allow"));
    }

    #[test]
    fn test_bash_other_command_allowed() {
        let input = HookInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(ToolInput {
                command: Some("cargo build".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let output = run_protect_config_hook(&input);
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("allow"));
    }

    #[test]
    fn test_read_config_allowed() {
        let input = HookInput {
            tool_name: Some("Read".to_string()),
            tool_input: Some(ToolInput {
                file_path: Some(".claude/reliability-config.yaml".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let output = run_protect_config_hook(&input);
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("allow"));
    }

    #[test]
    fn test_delete_jkw_session_blocked() {
        let input = HookInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(ToolInput {
                command: Some("rm .claude/jkw-session.local.md".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let output = run_protect_config_hook(&input);
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("block"));
        assert!(json.contains("JKW Session File"));
    }

    #[test]
    fn test_delete_jkw_state_blocked() {
        let input = HookInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(ToolInput {
                command: Some("rm .claude/jkw-state.local.yaml".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let output = run_protect_config_hook(&input);
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("block"));
    }

    #[test]
    fn test_no_tool_input_allowed() {
        let input = HookInput { tool_name: Some("Write".to_string()), ..Default::default() };

        let output = run_protect_config_hook(&input);
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("allow"));
    }
}
