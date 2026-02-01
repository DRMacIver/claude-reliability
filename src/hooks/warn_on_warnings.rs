//! Detect warnings in Bash tool stderr and create work items to track them.

use crate::hooks::post_tool_use::PostToolUseInput;
use crate::tasks::{Priority, SqliteTaskStore, TaskStore};
use std::path::Path;

/// Parsed command from Bash `tool_input`.
#[derive(serde::Deserialize)]
struct BashToolInput {
    command: Option<String>,
}

/// Parsed stderr from Bash `tool_response`.
#[derive(serde::Deserialize)]
struct BashToolResponse {
    stderr: Option<String>,
}

/// Environment variable to disable the warning detection hook.
const DISABLE_ENV_VAR: &str = "CLAUDE_RELIABILITY_DISABLE_HOOK";

/// Maximum length of warning text included in work item descriptions.
const MAX_WARNING_TEXT_LEN: usize = 2000;

/// Maximum length of command text included in work item titles.
const MAX_COMMAND_TITLE_LEN: usize = 60;

/// Extract all lines from `text` that contain "warning" (case-insensitive).
fn extract_warning_lines(text: &str) -> Vec<&str> {
    text.lines().filter(|line| line.to_ascii_lowercase().contains("warning")).collect()
}

/// Extract stderr text from the tool response.
///
/// Tries to parse as a JSON object with a `stderr` field first.
/// Falls back to treating the entire response as a plain string.
fn extract_stderr(tool_response: &serde_json::Value) -> Option<String> {
    // Try structured JSON with stderr field
    if let Ok(parsed) = serde_json::from_value::<BashToolResponse>(tool_response.clone()) {
        if let Some(stderr) = parsed.stderr {
            if !stderr.is_empty() {
                return Some(stderr);
            }
        }
        return None;
    }

    // Fall back to plain string
    if let Some(s) = tool_response.as_str() {
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }

    None
}

/// Check Bash tool output for warnings and create a work item if any are found.
///
/// # Errors
///
/// Returns an error if the task store cannot be opened or the task cannot be created.
pub fn check_bash_warnings(input: &PostToolUseInput, base_dir: &Path) -> Result<(), String> {
    if std::env::var(DISABLE_ENV_VAR).is_ok_and(|v| !v.is_empty()) {
        return Ok(());
    }

    let Some(response) = &input.tool_response else {
        return Ok(());
    };

    let Some(stderr) = extract_stderr(response) else {
        return Ok(());
    };

    let warning_lines = extract_warning_lines(&stderr);
    if warning_lines.is_empty() {
        return Ok(());
    }

    // Extract command from tool_input for context
    let command = input
        .tool_input
        .as_ref()
        .and_then(|v| serde_json::from_value::<BashToolInput>(v.clone()).ok())
        .and_then(|b| b.command)
        .unwrap_or_else(|| "<unknown command>".to_string());

    // Build truncated title
    let truncated_command = if command.len() > MAX_COMMAND_TITLE_LEN {
        format!("{}...", &command[..MAX_COMMAND_TITLE_LEN])
    } else {
        command.clone()
    };
    let title = format!("Fix warnings from: {truncated_command}");

    // Build description with command and warning lines
    let mut warning_text = warning_lines.join("\n");
    if warning_text.len() > MAX_WARNING_TEXT_LEN {
        warning_text.truncate(MAX_WARNING_TEXT_LEN);
        warning_text.push_str("\n... (truncated)");
    }
    let description =
        format!("Command:\n```\n{command}\n```\n\nWarnings:\n```\n{warning_text}\n```");

    let store = SqliteTaskStore::for_project(base_dir)
        .map_err(|e| format!("Failed to open task store: {e}"))?;

    store
        .create_task(&title, &description, Priority::Medium)
        .map_err(|e| format!("Failed to create warning task: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths;
    use crate::tasks::TaskFilter;
    use tempfile::TempDir;

    fn setup_db(dir: &Path) {
        let db_path = paths::project_db_path(dir);
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    }

    #[test]
    fn test_extract_warning_lines_finds_warnings() {
        let text = "compiling foo\nWarning: unused variable\nok\nwarning[E0123]: something\n";
        let lines = extract_warning_lines(text);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "Warning: unused variable");
        assert_eq!(lines[1], "warning[E0123]: something");
    }

    #[test]
    fn test_extract_warning_lines_no_warnings() {
        let text = "compiling foo\nerror: something\nok\n";
        let lines = extract_warning_lines(text);
        assert!(lines.is_empty());
    }

    #[test]
    fn test_extract_warning_lines_case_insensitive() {
        let text = "WARNING: loud\nwArNiNg: mixed case\n";
        let lines = extract_warning_lines(text);
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn test_extract_stderr_structured_json() {
        let response = serde_json::json!({"stdout": "ok", "stderr": "warning: unused"});
        let stderr = extract_stderr(&response);
        assert_eq!(stderr, Some("warning: unused".to_string()));
    }

    #[test]
    fn test_extract_stderr_empty_stderr() {
        let response = serde_json::json!({"stdout": "ok", "stderr": ""});
        let stderr = extract_stderr(&response);
        assert!(stderr.is_none());
    }

    #[test]
    fn test_extract_stderr_plain_string() {
        let response = serde_json::json!("warning: something went wrong");
        let stderr = extract_stderr(&response);
        assert_eq!(stderr, Some("warning: something went wrong".to_string()));
    }

    #[test]
    fn test_extract_stderr_no_stderr_field() {
        let response = serde_json::json!({"stdout": "ok"});
        let stderr = extract_stderr(&response);
        assert!(stderr.is_none());
    }

    #[test]
    fn test_extract_stderr_empty_plain_string() {
        let response = serde_json::json!("");
        let stderr = extract_stderr(&response);
        assert!(stderr.is_none());
    }

    #[test]
    #[serial_test::serial]
    fn test_check_bash_warnings_creates_work_item() {
        let dir = TempDir::new().unwrap();
        setup_db(dir.path());

        let input = PostToolUseInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({"command": "cargo build"})),
            tool_response: Some(serde_json::json!({
                "stdout": "Compiling foo",
                "stderr": "warning: unused variable `x`\nwarning: unused import"
            })),
        };

        let result = check_bash_warnings(&input, dir.path());
        assert!(result.is_ok());

        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 1);

        let task = &tasks[0];
        assert!(task.title.contains("Fix warnings from:"));
        assert!(task.title.contains("cargo build"));
        assert!(task.description.contains("cargo build"));
        assert!(task.description.contains("unused variable"));
        assert!(task.description.contains("unused import"));
    }

    #[test]
    #[serial_test::serial]
    fn test_check_bash_warnings_no_stderr_no_work_item() {
        let dir = TempDir::new().unwrap();
        setup_db(dir.path());

        let input = PostToolUseInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({"command": "cargo build"})),
            tool_response: Some(serde_json::json!({
                "stdout": "ok",
                "stderr": ""
            })),
        };

        let result = check_bash_warnings(&input, dir.path());
        assert!(result.is_ok());

        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_check_bash_warnings_no_warning_keyword_no_work_item() {
        let dir = TempDir::new().unwrap();
        setup_db(dir.path());

        let input = PostToolUseInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({"command": "cargo build"})),
            tool_response: Some(serde_json::json!({
                "stdout": "",
                "stderr": "error: could not compile\nnote: see above"
            })),
        };

        let result = check_bash_warnings(&input, dir.path());
        assert!(result.is_ok());

        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_check_bash_warnings_plain_string_response() {
        let dir = TempDir::new().unwrap();
        setup_db(dir.path());

        let input = PostToolUseInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({"command": "some-tool"})),
            tool_response: Some(serde_json::json!("warning: deprecated feature")),
        };

        let result = check_bash_warnings(&input, dir.path());
        assert!(result.is_ok());

        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].description.contains("deprecated feature"));
    }

    #[test]
    #[serial_test::serial]
    fn test_check_bash_warnings_missing_command() {
        let dir = TempDir::new().unwrap();
        setup_db(dir.path());

        let input = PostToolUseInput {
            tool_name: Some("Bash".to_string()),
            tool_input: None,
            tool_response: Some(serde_json::json!({
                "stdout": "",
                "stderr": "warning: something bad"
            })),
        };

        let result = check_bash_warnings(&input, dir.path());
        assert!(result.is_ok());

        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].title.contains("<unknown command>"));
    }

    #[test]
    #[serial_test::serial]
    fn test_check_bash_warnings_no_response() {
        let dir = TempDir::new().unwrap();

        let input = PostToolUseInput {
            tool_name: Some("Bash".to_string()),
            tool_input: None,
            tool_response: None,
        };

        let result = check_bash_warnings(&input, dir.path());
        assert!(result.is_ok());
    }

    #[test]
    #[serial_test::serial]
    fn test_check_bash_warnings_long_command_truncated_in_title() {
        let dir = TempDir::new().unwrap();
        setup_db(dir.path());

        let long_command = "a".repeat(200);
        let input = PostToolUseInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({"command": long_command})),
            tool_response: Some(serde_json::json!({
                "stdout": "",
                "stderr": "warning: something"
            })),
        };

        let result = check_bash_warnings(&input, dir.path());
        assert!(result.is_ok());

        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].title.contains("..."));
        // Title should have the truncated command, not the full one
        assert!(tasks[0].title.len() < 200);
        // But the description should have the full command
        assert!(tasks[0].description.contains(&long_command));
    }

    #[test]
    #[serial_test::serial]
    fn test_check_bash_warnings_disabled_by_env_var() {
        let dir = TempDir::new().unwrap();
        setup_db(dir.path());

        std::env::set_var(DISABLE_ENV_VAR, "1");

        let input = PostToolUseInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({"command": "cargo build"})),
            tool_response: Some(serde_json::json!({
                "stdout": "",
                "stderr": "warning: unused variable"
            })),
        };

        let result = check_bash_warnings(&input, dir.path());
        assert!(result.is_ok());

        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 0);

        std::env::remove_var(DISABLE_ENV_VAR);
    }

    #[test]
    #[serial_test::serial]
    fn test_check_bash_warnings_not_disabled_by_empty_env_var() {
        let dir = TempDir::new().unwrap();
        setup_db(dir.path());

        std::env::set_var(DISABLE_ENV_VAR, "");

        let input = PostToolUseInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({"command": "cargo build"})),
            tool_response: Some(serde_json::json!({
                "stdout": "",
                "stderr": "warning: unused variable"
            })),
        };

        let result = check_bash_warnings(&input, dir.path());
        assert!(result.is_ok());

        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 1);

        std::env::remove_var(DISABLE_ENV_VAR);
    }

    #[test]
    #[serial_test::serial]
    fn test_check_bash_warnings_long_warning_text_truncated() {
        let dir = TempDir::new().unwrap();
        setup_db(dir.path());

        // Create many warning lines that exceed MAX_WARNING_TEXT_LEN
        let mut stderr_lines = Vec::new();
        for i in 0..200 {
            stderr_lines.push(format!("warning: issue number {i} is very problematic"));
        }
        let stderr = stderr_lines.join("\n");

        let input = PostToolUseInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({"command": "make all"})),
            tool_response: Some(serde_json::json!({
                "stdout": "",
                "stderr": stderr
            })),
        };

        let result = check_bash_warnings(&input, dir.path());
        assert!(result.is_ok());

        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].description.contains("(truncated)"));
    }
}
