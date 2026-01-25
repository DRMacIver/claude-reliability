//! Debug subagent interaction logging.
//!
//! When `debug_logging` is enabled in the project config, every subagent
//! invocation is appended as a JSONL line to `.claude-reliability/subagent-events.jsonl`.
//! This allows debugging subagent behavior by inspecting prompts and responses.

use crate::config::ProjectConfig;
use crate::paths;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

/// Log file name within the data directory.
const SUBAGENT_EVENTS_FILE: &str = "subagent-events.jsonl";

/// Log a subagent invocation if debug logging is enabled.
///
/// Checks the project config for the `debug_logging` flag. If enabled,
/// appends a JSONL line containing the operation type, prompt, response,
/// and other metadata to the subagent events log file.
///
/// Errors are silently ignored — logging should never break subagent execution.
pub fn log_subagent_event(
    operation: &str,
    prompt: &str,
    response: Option<&str>,
    success: bool,
    duration_ms: Option<u64>,
) {
    log_subagent_event_in(operation, prompt, response, success, duration_ms, Path::new("."));
}

/// Log a subagent event in a specific base directory (for testing).
pub fn log_subagent_event_in(
    operation: &str,
    prompt: &str,
    response: Option<&str>,
    success: bool,
    duration_ms: Option<u64>,
    base_dir: &Path,
) {
    // Load config — if it fails, skip logging
    let Ok(Some(config)) = ProjectConfig::load_from(base_dir) else {
        return;
    };

    if !config.debug_logging {
        return;
    }

    write_subagent_event(operation, prompt, response, success, duration_ms, base_dir);
}

/// Write the subagent event to the log file.
fn write_subagent_event(
    operation: &str,
    prompt: &str,
    response: Option<&str>,
    success: bool,
    duration_ms: Option<u64>,
    base_dir: &Path,
) {
    let data_dir = paths::project_data_dir(base_dir);

    // Ensure data directory exists
    if std::fs::create_dir_all(&data_dir).is_err() {
        return;
    }

    let log_path = data_dir.join(SUBAGENT_EVENTS_FILE);

    let timestamp = chrono::Utc::now().to_rfc3339();

    let entry = serde_json::json!({
        "timestamp": timestamp,
        "operation": operation,
        "prompt": prompt,
        "response": response,
        "success": success,
        "duration_ms": duration_ms,
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
        let log_path = paths::project_data_dir(dir).join(SUBAGENT_EVENTS_FILE);
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
    fn test_log_subagent_event_when_enabled() {
        let dir = TempDir::new().unwrap();
        setup_config(dir.path(), true);

        log_subagent_event_in(
            "question_decision",
            "Should I answer this question?",
            Some("CONTINUE"),
            true,
            Some(1500),
            dir.path(),
        );

        let lines = read_log_lines(dir.path());
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["operation"], "question_decision");
        assert_eq!(lines[0]["prompt"], "Should I answer this question?");
        assert_eq!(lines[0]["response"], "CONTINUE");
        assert_eq!(lines[0]["success"], true);
        assert_eq!(lines[0]["duration_ms"], 1500);
        assert!(lines[0]["timestamp"].is_string());
    }

    #[test]
    fn test_log_subagent_event_when_disabled() {
        let dir = TempDir::new().unwrap();
        setup_config(dir.path(), false);

        log_subagent_event_in(
            "code_review",
            "Review this code",
            Some("APPROVE"),
            true,
            Some(5000),
            dir.path(),
        );

        let lines = read_log_lines(dir.path());
        assert!(lines.is_empty());
    }

    #[test]
    fn test_log_subagent_event_no_config() {
        let dir = TempDir::new().unwrap();
        // No config file at all

        log_subagent_event_in(
            "emergency_stop",
            "Should I allow stop?",
            Some("ACCEPT"),
            true,
            None,
            dir.path(),
        );

        let lines = read_log_lines(dir.path());
        assert!(lines.is_empty());
    }

    #[test]
    fn test_log_subagent_event_with_failure() {
        let dir = TempDir::new().unwrap();
        setup_config(dir.path(), true);

        log_subagent_event_in(
            "code_review",
            "Review this code",
            None,
            false,
            Some(60000),
            dir.path(),
        );

        let lines = read_log_lines(dir.path());
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["operation"], "code_review");
        assert_eq!(lines[0]["success"], false);
        assert!(lines[0]["response"].is_null());
    }

    #[test]
    fn test_log_subagent_event_multiple_events() {
        let dir = TempDir::new().unwrap();
        setup_config(dir.path(), true);

        log_subagent_event_in(
            "question_decision",
            "prompt1",
            Some("resp1"),
            true,
            Some(100),
            dir.path(),
        );
        log_subagent_event_in("code_review", "prompt2", Some("resp2"), true, Some(200), dir.path());
        log_subagent_event_in(
            "emergency_stop",
            "prompt3",
            Some("resp3"),
            true,
            Some(300),
            dir.path(),
        );

        let lines = read_log_lines(dir.path());
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0]["operation"], "question_decision");
        assert_eq!(lines[1]["operation"], "code_review");
        assert_eq!(lines[2]["operation"], "emergency_stop");
    }

    #[test]
    fn test_write_subagent_event_creates_data_dir() {
        let dir = TempDir::new().unwrap();
        let data_dir = paths::project_data_dir(dir.path());

        // Data dir should not exist yet
        assert!(!data_dir.exists());

        // Write event directly (bypassing config check)
        write_subagent_event("test", "prompt", Some("response"), true, None, dir.path());

        // Data dir should now exist
        assert!(data_dir.exists());

        let lines = read_log_lines(dir.path());
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn test_write_subagent_event_data_dir_creation_fails() {
        let dir = TempDir::new().unwrap();
        // Create a file where the data dir would go, so create_dir_all fails
        let data_dir = paths::project_data_dir(dir.path());
        std::fs::write(&data_dir, "blocking file").unwrap();

        // Should not panic, just silently skip
        write_subagent_event("test", "prompt", Some("response"), true, None, dir.path());

        // No log file created (data dir is a file, not a directory)
        assert!(!data_dir.join(SUBAGENT_EVENTS_FILE).exists());
    }

    #[test]
    fn test_write_subagent_event_file_open_fails() {
        let dir = TempDir::new().unwrap();
        let data_dir = paths::project_data_dir(dir.path());
        std::fs::create_dir_all(&data_dir).unwrap();

        // Create subagent-events.jsonl as a directory so file open fails
        let log_path = data_dir.join(SUBAGENT_EVENTS_FILE);
        std::fs::create_dir(&log_path).unwrap();

        // Should not panic, just silently skip
        write_subagent_event("test", "prompt", Some("response"), true, None, dir.path());
    }

    #[test]
    fn test_log_subagent_event_entry_format() {
        let dir = TempDir::new().unwrap();
        setup_config(dir.path(), true);

        log_subagent_event_in(
            "question_decision",
            "test prompt",
            Some("test response"),
            true,
            Some(1234),
            dir.path(),
        );

        let lines = read_log_lines(dir.path());
        assert_eq!(lines.len(), 1);

        let entry = &lines[0];
        // Must have all fields
        assert!(entry.get("timestamp").is_some());
        assert!(entry.get("operation").is_some());
        assert!(entry.get("prompt").is_some());
        assert!(entry.get("response").is_some());
        assert!(entry.get("success").is_some());
        assert!(entry.get("duration_ms").is_some());

        // Timestamp should be ISO 8601 / RFC 3339
        let ts = entry["timestamp"].as_str().unwrap();
        assert!(chrono::DateTime::parse_from_rfc3339(ts).is_ok());
    }
}
