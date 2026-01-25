//! Pattern-matched reminders that trigger when the assistant uses certain phrases.
//!
//! Reminders are defined in `.claude-reliability/reminders.yaml` and are shown
//! as non-blocking context when the assistant's output matches a pattern.

use crate::error::Result;
use crate::paths::project_data_dir;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::SystemTime;

/// The filename for the reminders configuration.
const REMINDERS_FILENAME: &str = "reminders.yaml";

/// Configuration for a single reminder with multiple patterns.
#[derive(Debug, Clone, Deserialize)]
pub struct ReminderConfig {
    /// The reminder message to display when a pattern matches.
    pub message: String,
    /// The patterns that trigger this reminder (case-insensitive regex).
    pub patterns: Vec<String>,
}

/// Top-level configuration for reminders.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct RemindersConfig {
    /// The list of reminders.
    #[serde(default)]
    pub reminders: Vec<ReminderConfig>,
}

/// A compiled reminder with pre-compiled regex patterns.
#[derive(Debug)]
struct CompiledReminder {
    /// The reminder message.
    message: String,
    /// The compiled regex patterns.
    patterns: Vec<Regex>,
}

/// Cache for compiled reminders to avoid re-parsing regex on each call.
#[derive(Debug, Default)]
struct RemindersCache {
    /// The cached compiled reminders.
    reminders: Vec<CompiledReminder>,
    /// The path to the file that was loaded.
    file_path: Option<PathBuf>,
    /// The mtime of the file when it was last loaded.
    mtime: Option<SystemTime>,
}

/// Global cache for compiled reminders.
static REMINDERS_CACHE: Lazy<RwLock<RemindersCache>> =
    Lazy::new(|| RwLock::new(RemindersCache::default()));

/// Get the path to the reminders configuration file.
///
/// Returns `<base_dir>/.claude-reliability/reminders.yaml`.
#[must_use]
pub fn reminders_path(base_dir: &Path) -> PathBuf {
    project_data_dir(base_dir).join(REMINDERS_FILENAME)
}

/// Load reminders configuration from the project directory.
///
/// Returns an empty configuration if the file doesn't exist.
/// Returns an error if the file exists but has invalid YAML.
///
/// # Errors
///
/// Returns an error if:
/// - The file exists but cannot be read
/// - The file contains invalid YAML
pub fn load_reminders(base_dir: &Path) -> Result<RemindersConfig> {
    let path = reminders_path(base_dir);

    if !path.exists() {
        return Ok(RemindersConfig::default());
    }

    let content = fs::read_to_string(&path)?;
    let config: RemindersConfig = serde_yaml::from_str(&content)?;
    Ok(config)
}

/// Compile a single reminder configuration into a compiled reminder.
///
/// All patterns get `(?i)` prefix for case-insensitivity.
fn compile_reminder(config: &ReminderConfig) -> Result<CompiledReminder> {
    let mut patterns = Vec::with_capacity(config.patterns.len());

    for pattern in &config.patterns {
        // Add case-insensitive flag
        let pattern_with_flag = format!("(?i){pattern}");
        let regex = Regex::new(&pattern_with_flag)?;
        patterns.push(regex);
    }

    Ok(CompiledReminder { message: config.message.clone(), patterns })
}

/// Compile a reminders configuration into compiled reminders.
///
/// # Errors
///
/// Returns an error if any pattern is an invalid regex.
fn compile_reminders(config: &RemindersConfig) -> Result<Vec<CompiledReminder>> {
    config.reminders.iter().map(compile_reminder).collect()
}

/// Check if the cache is still valid.
fn is_cache_valid(cache: &RemindersCache, path: &Path) -> bool {
    // Check if the cached path matches
    let Some(cached_path) = &cache.file_path else {
        return false;
    };
    if cached_path != path {
        return false;
    }

    // Check if the file still exists and has the same mtime
    let Some(cached_mtime) = cache.mtime else {
        // Cache was for a non-existent file
        return !path.exists();
    };

    // File existed when cached, check if it still has the same mtime
    path.metadata().ok().and_then(|m| m.modified().ok()).is_some_and(|m| m == cached_mtime)
}

/// Get compiled reminders, using the cache if valid.
///
/// # Errors
///
/// Returns an error if the file has invalid YAML or regex patterns.
fn get_compiled_reminders(base_dir: &Path) -> Result<Vec<CompiledReminder>> {
    let path = reminders_path(base_dir);

    // Try to read from cache first
    {
        let cache = REMINDERS_CACHE.read().unwrap();
        if is_cache_valid(&cache, &path) {
            return Ok(cache
                .reminders
                .iter()
                .map(|r| CompiledReminder {
                    message: r.message.clone(),
                    patterns: r.patterns.clone(),
                })
                .collect());
        }
    }

    // Cache miss or invalid, need to reload
    let config = load_reminders(base_dir)?;
    let compiled = compile_reminders(&config)?;

    // Get the mtime before updating cache
    let mtime = path.metadata().ok().and_then(|m| m.modified().ok());

    // Update cache
    {
        let mut cache = REMINDERS_CACHE.write().unwrap();
        cache.reminders = compiled
            .iter()
            .map(|r| CompiledReminder { message: r.message.clone(), patterns: r.patterns.clone() })
            .collect();
        cache.file_path = Some(path);
        cache.mtime = mtime;
    }

    Ok(compiled)
}

/// Check text against reminders and return any matching reminder messages.
///
/// This function checks the given text against all configured reminder patterns
/// and returns the messages for any reminders whose patterns match.
///
/// # Arguments
///
/// * `text` - The text to check against reminder patterns.
/// * `base_dir` - The project base directory where reminders.yaml is located.
///
/// # Returns
///
/// A vector of reminder messages for all matching patterns. Returns an empty
/// vector if:
/// - No patterns match
/// - The reminders file doesn't exist
/// - There's an error loading the reminders (errors are logged to stderr)
pub fn check_reminders(text: &str, base_dir: &Path) -> Vec<String> {
    let compiled = match get_compiled_reminders(base_dir) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Warning: Failed to load reminders: {e}");
            return Vec::new();
        }
    };

    let mut messages = Vec::new();

    for reminder in &compiled {
        for pattern in &reminder.patterns {
            if pattern.is_match(text) {
                messages.push(reminder.message.clone());
                // Only add each reminder message once, even if multiple patterns match
                break;
            }
        }
    }

    messages
}

/// Clear the reminders cache. Useful for testing.
///
/// # Panics
///
/// Panics if the cache lock is poisoned.
#[cfg(test)]
pub fn clear_cache() {
    let mut cache = REMINDERS_CACHE.write().unwrap();
    *cache = RemindersCache::default();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::time::Duration;
    use tempfile::TempDir;

    fn setup_test_dir() -> TempDir {
        clear_cache();
        TempDir::new().unwrap()
    }

    fn create_reminders_file(dir: &Path, content: &str) {
        let data_dir = project_data_dir(dir);
        fs::create_dir_all(&data_dir).unwrap();
        let path = reminders_path(dir);
        let mut file = File::create(path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn test_load_reminders_valid_yaml() {
        let dir = setup_test_dir();
        create_reminders_file(
            dir.path(),
            r#"
reminders:
  - message: "Handle edge cases"
    patterns:
      - "edge case"
      - "corner case"
  - message: "Don't disable features"
    patterns:
      - "disable"
"#,
        );

        let config = load_reminders(dir.path()).unwrap();
        assert_eq!(config.reminders.len(), 2);
        assert_eq!(config.reminders[0].message, "Handle edge cases");
        assert_eq!(config.reminders[0].patterns.len(), 2);
        assert_eq!(config.reminders[1].message, "Don't disable features");
        assert_eq!(config.reminders[1].patterns.len(), 1);
    }

    #[test]
    fn test_load_reminders_missing_file() {
        let dir = setup_test_dir();
        // Don't create the file

        let config = load_reminders(dir.path()).unwrap();
        assert!(config.reminders.is_empty());
    }

    #[test]
    fn test_load_reminders_invalid_yaml() {
        let dir = setup_test_dir();
        create_reminders_file(
            dir.path(),
            r"
this is not valid yaml: {{{
",
        );

        let result = load_reminders(dir.path());
        assert!(result.is_err());
        let err_str = format!("{:?}", result.unwrap_err());
        assert!(err_str.contains("Yaml"));
    }

    #[test]
    fn test_check_reminders_single_match() {
        let dir = setup_test_dir();
        create_reminders_file(
            dir.path(),
            r#"
reminders:
  - message: "Handle edge cases"
    patterns:
      - "edge case"
"#,
        );

        let messages = check_reminders("We should consider this edge case", dir.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0], "Handle edge cases");
    }

    #[test]
    fn test_check_reminders_multiple_patterns_same_reminder() {
        let dir = setup_test_dir();
        create_reminders_file(
            dir.path(),
            r#"
reminders:
  - message: "Handle edge cases"
    patterns:
      - "edge case"
      - "corner case"
"#,
        );

        // Should only get one message even if both patterns match
        let messages = check_reminders("This edge case is also a corner case", dir.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0], "Handle edge cases");
    }

    #[test]
    fn test_check_reminders_case_insensitive() {
        let dir = setup_test_dir();
        create_reminders_file(
            dir.path(),
            r#"
reminders:
  - message: "Handle edge cases"
    patterns:
      - "edge case"
"#,
        );

        let messages = check_reminders("EDGE CASE should be handled", dir.path());
        assert_eq!(messages.len(), 1);

        let messages = check_reminders("Edge Case matters", dir.path());
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn test_check_reminders_no_match() {
        let dir = setup_test_dir();
        create_reminders_file(
            dir.path(),
            r#"
reminders:
  - message: "Handle edge cases"
    patterns:
      - "edge case"
"#,
        );

        let messages = check_reminders("This is a normal scenario", dir.path());
        assert!(messages.is_empty());
    }

    #[test]
    fn test_compile_reminders_invalid_regex() {
        let config = RemindersConfig {
            reminders: vec![ReminderConfig {
                message: "Test".to_string(),
                patterns: vec!["[invalid".to_string()], // Invalid regex
            }],
        };

        let result = compile_reminders(&config);
        assert!(result.is_err());
        let err_str = format!("{:?}", result.unwrap_err());
        assert!(err_str.contains("Regex"));
    }

    #[test]
    fn test_cache_invalidates_on_file_change() {
        let dir = setup_test_dir();
        create_reminders_file(
            dir.path(),
            r#"
reminders:
  - message: "First message"
    patterns:
      - "first"
"#,
        );

        // First check should load and cache
        let messages = check_reminders("first pattern", dir.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0], "First message");

        // Wait a bit to ensure different mtime
        std::thread::sleep(Duration::from_millis(10));

        // Modify the file
        create_reminders_file(
            dir.path(),
            r#"
reminders:
  - message: "Second message"
    patterns:
      - "second"
"#,
        );

        // Should reload due to mtime change
        let messages = check_reminders("second pattern", dir.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0], "Second message");

        // Old pattern should no longer match
        let messages = check_reminders("first pattern", dir.path());
        assert!(messages.is_empty());
    }

    #[test]
    fn test_reminders_path() {
        let dir = Path::new("/some/project");
        let path = reminders_path(dir);
        assert_eq!(path, PathBuf::from("/some/project/.claude-reliability/reminders.yaml"));
    }

    #[test]
    fn test_check_reminders_multiple_reminders_match() {
        let dir = setup_test_dir();
        create_reminders_file(
            dir.path(),
            r#"
reminders:
  - message: "Handle edge cases"
    patterns:
      - "edge case"
  - message: "Don't disable features"
    patterns:
      - "disable"
"#,
        );

        let messages = check_reminders("We should disable this edge case handler", dir.path());
        assert_eq!(messages.len(), 2);
        assert!(messages.contains(&"Handle edge cases".to_string()));
        assert!(messages.contains(&"Don't disable features".to_string()));
    }

    #[test]
    fn test_check_reminders_regex_patterns() {
        let dir = setup_test_dir();
        create_reminders_file(
            dir.path(),
            r#"
reminders:
  - message: "Word boundary test"
    patterns:
      - '\btest\b'
"#,
        );

        // Should match 'test' as a word
        let messages = check_reminders("this is a test case", dir.path());
        assert_eq!(messages.len(), 1);

        // Should not match 'testing' (test is not a whole word)
        let messages = check_reminders("testing in progress", dir.path());
        assert!(messages.is_empty());
    }

    #[test]
    fn test_check_reminders_empty_text() {
        let dir = setup_test_dir();
        create_reminders_file(
            dir.path(),
            r#"
reminders:
  - message: "Test"
    patterns:
      - "test"
"#,
        );

        let messages = check_reminders("", dir.path());
        assert!(messages.is_empty());
    }

    #[test]
    fn test_check_reminders_missing_file_returns_empty() {
        let dir = setup_test_dir();
        // Don't create the file

        let messages = check_reminders("any text", dir.path());
        assert!(messages.is_empty());
    }

    #[test]
    fn test_cache_valid_when_file_doesnt_exist() {
        let dir = setup_test_dir();
        // Don't create file, but call check_reminders to populate cache

        let messages = check_reminders("text", dir.path());
        assert!(messages.is_empty());

        // Second call should use cache
        let messages = check_reminders("text", dir.path());
        assert!(messages.is_empty());

        // Now create the file
        create_reminders_file(
            dir.path(),
            r#"
reminders:
  - message: "New message"
    patterns:
      - "text"
"#,
        );

        // Should detect file was created and reload
        let messages = check_reminders("text", dir.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0], "New message");
    }

    #[test]
    fn test_cache_invalidates_on_path_change() {
        let dir1 = setup_test_dir();
        let dir2 = TempDir::new().unwrap();

        create_reminders_file(
            dir1.path(),
            r#"
reminders:
  - message: "Dir 1 message"
    patterns:
      - "test"
"#,
        );

        create_reminders_file(
            dir2.path(),
            r#"
reminders:
  - message: "Dir 2 message"
    patterns:
      - "test"
"#,
        );

        // Load from dir1
        let messages = check_reminders("test", dir1.path());
        assert_eq!(messages[0], "Dir 1 message");

        // Load from dir2 should not use dir1's cache
        let messages = check_reminders("test", dir2.path());
        assert_eq!(messages[0], "Dir 2 message");
    }

    #[test]
    fn test_compile_reminder_adds_case_insensitive_flag() {
        let config =
            ReminderConfig { message: "Test".to_string(), patterns: vec!["Test".to_string()] };

        let compiled = compile_reminder(&config).unwrap();
        // The pattern should match case-insensitively
        assert!(compiled.patterns[0].is_match("test"));
        assert!(compiled.patterns[0].is_match("TEST"));
        assert!(compiled.patterns[0].is_match("TeSt"));
    }

    #[test]
    fn test_load_reminders_empty_yaml() {
        let dir = setup_test_dir();
        create_reminders_file(dir.path(), "");

        let config = load_reminders(dir.path()).unwrap();
        assert!(config.reminders.is_empty());
    }

    #[test]
    fn test_load_reminders_empty_reminders_list() {
        let dir = setup_test_dir();
        create_reminders_file(dir.path(), "reminders: []");

        let config = load_reminders(dir.path()).unwrap();
        assert!(config.reminders.is_empty());
    }

    #[test]
    fn test_check_reminders_logs_error_on_invalid_regex() {
        let dir = setup_test_dir();
        create_reminders_file(
            dir.path(),
            r#"
reminders:
  - message: "Test"
    patterns:
      - "[invalid regex"
"#,
        );

        // Should return empty vec and log error (not panic)
        let messages = check_reminders("any text", dir.path());
        assert!(messages.is_empty());
    }

    #[test]
    fn test_check_reminders_logs_error_on_invalid_yaml() {
        let dir = setup_test_dir();
        create_reminders_file(
            dir.path(),
            r"
invalid: yaml: {{{
",
        );

        // Should return empty vec and log error (not panic)
        let messages = check_reminders("any text", dir.path());
        assert!(messages.is_empty());
    }
}
