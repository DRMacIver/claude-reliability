//! Session file parsing for autonomous mode.
//!
//! The session file (`.claude/autonomous-session.local.md`) tracks the state
//! of an autonomous development session using YAML frontmatter.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Default path for the session file.
pub const SESSION_FILE_PATH: &str = ".claude/autonomous-session.local.md";

/// Staleness threshold - iterations without issue changes before stopping.
pub const STALENESS_THRESHOLD: u32 = 5;

/// Session configuration stored in the YAML frontmatter.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionConfig {
    /// Current iteration number.
    #[serde(default)]
    pub iteration: u32,
    /// Iteration when issues last changed.
    #[serde(default)]
    pub last_issue_change_iteration: u32,
    /// Snapshot of issue IDs from the last check.
    #[serde(default)]
    pub issue_snapshot: Vec<String>,
}

impl SessionConfig {
    /// Get the issue snapshot as a `HashSet`.
    #[must_use]
    pub fn issue_snapshot_set(&self) -> HashSet<String> {
        self.issue_snapshot.iter().cloned().collect()
    }

    /// Calculate iterations since the last issue change.
    #[must_use]
    pub const fn iterations_since_change(&self) -> u32 {
        self.iteration.saturating_sub(self.last_issue_change_iteration)
    }

    /// Check if the session is stale (no progress for too long).
    #[must_use]
    pub const fn is_stale(&self) -> bool {
        self.iterations_since_change() >= STALENESS_THRESHOLD
    }
}

/// Parse a session file with YAML frontmatter.
///
/// The file format is:
/// ```text
/// ---
/// iteration: 5
/// last_issue_change_iteration: 3
/// issue_snapshot:
///   - project-123
///   - project-456
/// ---
///
/// # Session Log
/// ...
/// ```
///
/// # Errors
///
/// Returns an error if the file cannot be read or parsed.
pub fn parse_session_file(path: &Path) -> Result<Option<SessionConfig>> {
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(path)?;

    // Check for YAML frontmatter
    if !content.starts_with("---") {
        return Err(Error::InvalidSessionFile(
            "Session file must start with YAML frontmatter (---)".to_string(),
        ));
    }

    // Split on --- to get the frontmatter
    let parts: Vec<&str> = content.splitn(3, "---").collect();
    if parts.len() < 3 {
        return Err(Error::InvalidSessionFile("Invalid YAML frontmatter format".to_string()));
    }

    // Parse the YAML
    let yaml_content = parts[1].trim();
    let config: SessionConfig = serde_yaml::from_str(yaml_content)?;

    Ok(Some(config))
}

/// Write a session file with YAML frontmatter.
///
/// # Errors
///
/// Returns an error if the file cannot be written.
pub fn write_session_file(path: &Path, config: &SessionConfig) -> Result<()> {
    let yaml = serde_yaml::to_string(config)?;
    let content = format!(
        "---\n{yaml}---\n\n# Autonomous Session Log\n\nThis file tracks the autonomous development session.\n"
    );

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(path, content)?;
    Ok(())
}

/// Delete the session file if it exists.
///
/// # Errors
///
/// Returns an error if the file cannot be removed.
pub fn cleanup_session_file(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_parse_session_file_not_exists() {
        let result = parse_session_file(Path::new("/nonexistent/file.md")).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_session_file_valid() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.md");
        fs::write(
            &path,
            r"---
iteration: 5
last_issue_change_iteration: 3
issue_snapshot:
  - project-123
  - project-456
---

# Log
",
        )
        .unwrap();

        let config = parse_session_file(&path).unwrap().unwrap();
        assert_eq!(config.iteration, 5);
        assert_eq!(config.last_issue_change_iteration, 3);
        assert_eq!(config.issue_snapshot, vec!["project-123", "project-456"]);
    }

    #[test]
    fn test_parse_session_file_empty_snapshot() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.md");
        fs::write(
            &path,
            r"---
iteration: 1
---

# Log
",
        )
        .unwrap();

        let config = parse_session_file(&path).unwrap().unwrap();
        assert_eq!(config.iteration, 1);
        assert_eq!(config.last_issue_change_iteration, 0);
        assert!(config.issue_snapshot.is_empty());
    }

    #[test]
    fn test_parse_session_file_no_frontmatter() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.md");
        fs::write(&path, "Just some content without frontmatter").unwrap();

        let result = parse_session_file(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_write_session_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.md");

        let config = SessionConfig {
            iteration: 3,
            last_issue_change_iteration: 2,
            issue_snapshot: vec!["issue-1".to_string(), "issue-2".to_string()],
        };

        write_session_file(&path, &config).unwrap();

        // Read it back
        let parsed = parse_session_file(&path).unwrap().unwrap();
        assert_eq!(parsed.iteration, 3);
        assert_eq!(parsed.issue_snapshot, vec!["issue-1", "issue-2"]);
    }

    #[test]
    fn test_cleanup_session_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.md");
        fs::write(&path, "content").unwrap();

        assert!(path.exists());
        cleanup_session_file(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn test_cleanup_session_file_not_exists() {
        let path = Path::new("/nonexistent/file.md");
        // Should not error
        cleanup_session_file(path).unwrap();
    }

    #[test]
    fn test_session_config_iterations_since_change() {
        let config =
            SessionConfig { iteration: 10, last_issue_change_iteration: 7, issue_snapshot: vec![] };
        assert_eq!(config.iterations_since_change(), 3);
    }

    #[test]
    fn test_session_config_is_stale() {
        let stale =
            SessionConfig { iteration: 10, last_issue_change_iteration: 4, issue_snapshot: vec![] };
        assert!(stale.is_stale()); // 6 iterations since change

        let not_stale =
            SessionConfig { iteration: 10, last_issue_change_iteration: 8, issue_snapshot: vec![] };
        assert!(!not_stale.is_stale()); // 2 iterations since change
    }

    #[test]
    fn test_session_config_issue_snapshot_set() {
        let config = SessionConfig {
            iteration: 1,
            last_issue_change_iteration: 1,
            issue_snapshot: vec!["a".to_string(), "b".to_string(), "a".to_string()],
        };
        let set = config.issue_snapshot_set();
        assert_eq!(set.len(), 2);
        assert!(set.contains("a"));
        assert!(set.contains("b"));
    }

    #[test]
    fn test_staleness_threshold() {
        assert_eq!(STALENESS_THRESHOLD, 5);
    }

    #[test]
    fn test_parse_session_invalid_frontmatter_format() {
        // Test with frontmatter that doesn't have proper closing ---
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.md");

        // Write a file with only opening --- but no closing
        fs::write(&path, "---\niteration: 1\n# No closing delimiter").unwrap();

        let result = parse_session_file(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_write_session_creates_parent_directory() {
        let dir = TempDir::new().unwrap();
        // Path with non-existent parent directory
        let nested = dir.path().join("deeply").join("nested").join("path");
        let path = nested.join("session.md");

        // Verify parent doesn't exist yet
        assert!(!nested.exists());

        let config =
            SessionConfig { iteration: 1, last_issue_change_iteration: 1, issue_snapshot: vec![] };

        write_session_file(&path, &config).unwrap();

        // Verify both parent and file now exist
        assert!(nested.exists());
        assert!(path.exists());
    }
}
