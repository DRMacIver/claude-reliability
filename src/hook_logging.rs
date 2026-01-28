//! Debug hook event logging.
//!
//! When `debug_logging` is enabled in the project config, every hook
//! invocation is appended as a JSONL line to `.claude-reliability/hook-events.jsonl`.
//! This allows debugging hook behavior by inspecting exactly what events were received.

use crate::config::ProjectConfig;
use crate::paths;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

/// Log file name within the data directory.
const HOOK_EVENTS_FILE: &str = "hook-events.jsonl";

/// Log a hook event if debug logging is enabled.
///
/// Checks the project config for the `debug_logging` flag. If enabled,
/// appends a JSONL line containing the hook type, timestamp, and raw input
/// to the hook events log file.
///
/// Errors are silently ignored — logging should never break hook execution.
#[cfg(feature = "cli")]
pub fn log_hook_event(hook_type: &str, raw_input: &str) {
    log_hook_event_in(hook_type, raw_input, Path::new("."));
}

/// Log a hook event in a specific base directory (for testing).
pub fn log_hook_event_in(hook_type: &str, raw_input: &str, base_dir: &Path) {
    // Load config — if it fails, skip logging
    let Ok(Some(config)) = ProjectConfig::load_from(base_dir) else {
        return;
    };

    if !config.debug_logging {
        return;
    }

    write_hook_event(hook_type, raw_input, base_dir);
}

/// Write the hook event to the log file.
fn write_hook_event(hook_type: &str, raw_input: &str, base_dir: &Path) {
    let data_dir = paths::project_data_dir(base_dir);

    // Ensure data directory exists
    if std::fs::create_dir_all(&data_dir).is_err() {
        return;
    }

    let log_path = data_dir.join(HOOK_EVENTS_FILE);

    let timestamp = chrono::Utc::now().to_rfc3339();

    // Parse the raw input as JSON to embed it properly, or store as string if invalid
    let input_value: serde_json::Value = serde_json::from_str(raw_input)
        .unwrap_or_else(|_| serde_json::Value::String(raw_input.to_string()));

    let entry = serde_json::json!({
        "timestamp": timestamp,
        "hook_type": hook_type,
        "input": input_value,
    });

    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&log_path) else {
        return;
    };

    // Write the entry as a single line
    let _ = writeln!(file, "{entry}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_config(dir: &Path, debug_logging: bool) {
        let config = ProjectConfig { debug_logging, ..Default::default() };
        config.save_to(dir).unwrap();
    }

    fn read_log_lines(dir: &Path) -> Vec<serde_json::Value> {
        let log_path = paths::project_data_dir(dir).join(HOOK_EVENTS_FILE);
        if !log_path.exists() {
            return vec![];
        }
        let content = std::fs::read_to_string(&log_path).unwrap();
        content
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn test_log_hook_event_when_enabled() {
        let dir = TempDir::new().unwrap();
        setup_config(dir.path(), true);

        let input = r#"{"tool_name": "Read", "tool_input": {"file_path": "test.rs"}}"#;
        log_hook_event_in("pre-tool-use", input, dir.path());

        let lines = read_log_lines(dir.path());
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["hook_type"], "pre-tool-use");
        assert!(lines[0]["timestamp"].is_string());
        assert_eq!(lines[0]["input"]["tool_name"], "Read");
    }

    #[test]
    fn test_log_hook_event_when_disabled() {
        let dir = TempDir::new().unwrap();
        setup_config(dir.path(), false);

        log_hook_event_in("stop", "{}", dir.path());

        let lines = read_log_lines(dir.path());
        assert!(lines.is_empty());
    }

    #[test]
    fn test_log_hook_event_no_config() {
        let dir = TempDir::new().unwrap();
        // No config file at all

        log_hook_event_in("stop", "{}", dir.path());

        let lines = read_log_lines(dir.path());
        assert!(lines.is_empty());
    }

    #[test]
    fn test_log_hook_event_multiple_events() {
        let dir = TempDir::new().unwrap();
        setup_config(dir.path(), true);

        log_hook_event_in("stop", r#"{"transcript": []}"#, dir.path());
        log_hook_event_in("pre-tool-use", r#"{"tool_name": "Bash"}"#, dir.path());
        log_hook_event_in("user-prompt-submit", r#"{"prompt": "hello"}"#, dir.path());

        let lines = read_log_lines(dir.path());
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0]["hook_type"], "stop");
        assert_eq!(lines[1]["hook_type"], "pre-tool-use");
        assert_eq!(lines[2]["hook_type"], "user-prompt-submit");
    }

    #[test]
    fn test_log_hook_event_invalid_json_input() {
        let dir = TempDir::new().unwrap();
        setup_config(dir.path(), true);

        log_hook_event_in("stop", "not valid json", dir.path());

        let lines = read_log_lines(dir.path());
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["hook_type"], "stop");
        // Invalid JSON should be stored as a string
        assert_eq!(lines[0]["input"], "not valid json");
    }

    #[test]
    fn test_log_hook_event_empty_input() {
        let dir = TempDir::new().unwrap();
        setup_config(dir.path(), true);

        log_hook_event_in("post-tool-use", "", dir.path());

        let lines = read_log_lines(dir.path());
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["hook_type"], "post-tool-use");
        assert_eq!(lines[0]["input"], "");
    }

    #[test]
    fn test_write_hook_event_creates_data_dir() {
        let dir = TempDir::new().unwrap();
        let data_dir = paths::project_data_dir(dir.path());

        // Data dir should not exist yet
        assert!(!data_dir.exists());

        // Write event directly (bypassing config check)
        write_hook_event("test", "{}", dir.path());

        // Data dir should now exist
        assert!(data_dir.exists());

        let lines = read_log_lines(dir.path());
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn test_write_hook_event_data_dir_creation_fails() {
        let dir = TempDir::new().unwrap();
        // Create a file where the data dir would go, so create_dir_all fails
        let data_dir = paths::project_data_dir(dir.path());
        std::fs::write(&data_dir, "blocking file").unwrap();

        // Should not panic, just silently skip
        write_hook_event("test", "{}", dir.path());

        // No log file created (data dir is a file, not a directory)
        assert!(!data_dir.join(HOOK_EVENTS_FILE).exists());
    }

    #[test]
    fn test_write_hook_event_file_open_fails() {
        let dir = TempDir::new().unwrap();
        let data_dir = paths::project_data_dir(dir.path());
        std::fs::create_dir_all(&data_dir).unwrap();

        // Create hook-events.jsonl as a directory so file open fails
        let log_path = data_dir.join(HOOK_EVENTS_FILE);
        std::fs::create_dir(&log_path).unwrap();

        // Should not panic, just silently skip
        write_hook_event("test", "{}", dir.path());
    }

    #[test]
    fn test_log_hook_event_entry_format() {
        let dir = TempDir::new().unwrap();
        setup_config(dir.path(), true);

        let input = r#"{"key": "value"}"#;
        log_hook_event_in("stop", input, dir.path());

        let lines = read_log_lines(dir.path());
        assert_eq!(lines.len(), 1);

        let entry = &lines[0];
        // Must have all three fields
        assert!(entry.get("timestamp").is_some());
        assert!(entry.get("hook_type").is_some());
        assert!(entry.get("input").is_some());

        // Timestamp should be ISO 8601 / RFC 3339
        let ts = entry["timestamp"].as_str().unwrap();
        assert!(chrono::DateTime::parse_from_rfc3339(ts).is_ok());
    }
}
