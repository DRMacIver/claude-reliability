//! Session file parsing for just-keep-working mode.
//!
//! Session state is split into two files:
//! - `.claude/jkw-state.local.yaml` - Hook-managed YAML state (iteration, staleness)
//! - `.claude/jkw-session.local.md` - LLM-editable markdown (session notes, goals)
//!
//! This separation prevents the LLM from accidentally breaking the YAML
//! state when editing session notes.

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Path for hook-managed session state (YAML).
pub const SESSION_STATE_PATH: &str = ".claude/jkw-state.local.yaml";

/// Path for LLM-editable session notes (markdown).
pub const SESSION_NOTES_PATH: &str = ".claude/jkw-session.local.md";

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
    /// Hash of git diff for staleness detection when beads is not available.
    /// This provides a fallback mechanism to detect progress via code changes.
    #[serde(default)]
    pub git_diff_hash: Option<String>,
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

/// Parse the session state file (YAML).
///
/// The file format is plain YAML:
/// ```yaml
/// iteration: 5
/// last_issue_change_iteration: 3
/// issue_snapshot:
///   - project-123
///   - project-456
/// ```
///
/// # Errors
///
/// Returns an error if the file cannot be read or parsed.
pub fn parse_session_state(path: &Path) -> Result<Option<SessionConfig>> {
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(path)?;
    let config: SessionConfig = serde_yaml::from_str(&content)?;

    Ok(Some(config))
}

/// Write the session state file (YAML).
///
/// # Errors
///
/// Returns an error if the file cannot be written.
pub fn write_session_state(path: &Path, config: &SessionConfig) -> Result<()> {
    let yaml = serde_yaml::to_string(config)?;

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(path, yaml)?;
    Ok(())
}

/// Delete the session state file if it exists.
///
/// # Errors
///
/// Returns an error if the file cannot be removed.
pub fn cleanup_session_state(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

/// Delete both session files (state and notes) if they exist.
///
/// # Errors
///
/// Returns an error if files cannot be removed.
pub fn cleanup_session_files(base_dir: &Path) -> Result<()> {
    let state_path = base_dir.join(SESSION_STATE_PATH);
    let notes_path = base_dir.join(SESSION_NOTES_PATH);

    if state_path.exists() {
        fs::remove_file(state_path)?;
    }
    if notes_path.exists() {
        fs::remove_file(notes_path)?;
    }
    Ok(())
}

/// Default path for the problem mode marker file.
pub const PROBLEM_MODE_MARKER_PATH: &str = ".claude/problem-mode.local";

/// Check if problem mode is active (marker file exists).
#[must_use]
pub fn is_problem_mode_active(base_dir: &Path) -> bool {
    base_dir.join(PROBLEM_MODE_MARKER_PATH).exists()
}

/// Enter problem mode by creating the marker file.
///
/// # Errors
///
/// Returns an error if the marker file cannot be created.
pub fn enter_problem_mode(base_dir: &Path) -> Result<()> {
    let marker_path = base_dir.join(PROBLEM_MODE_MARKER_PATH);

    // Ensure parent directory exists
    if let Some(parent) = marker_path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(&marker_path, "Problem mode active - tool use blocked until next stop")?;
    Ok(())
}

/// Exit problem mode by removing the marker file.
///
/// # Errors
///
/// Returns an error if the marker file cannot be removed.
pub fn exit_problem_mode(base_dir: &Path) -> Result<()> {
    let marker_path = base_dir.join(PROBLEM_MODE_MARKER_PATH);
    if marker_path.exists() {
        fs::remove_file(marker_path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_parse_session_state_not_exists() {
        let result = parse_session_state(Path::new("/nonexistent/file.yaml")).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_session_state_valid() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.yaml");
        fs::write(
            &path,
            r"iteration: 5
last_issue_change_iteration: 3
issue_snapshot:
  - project-123
  - project-456
",
        )
        .unwrap();

        let config = parse_session_state(&path).unwrap().unwrap();
        assert_eq!(config.iteration, 5);
        assert_eq!(config.last_issue_change_iteration, 3);
        assert_eq!(config.issue_snapshot, vec!["project-123", "project-456"]);
    }

    #[test]
    fn test_parse_session_state_empty_snapshot() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.yaml");
        fs::write(
            &path,
            r"iteration: 1
",
        )
        .unwrap();

        let config = parse_session_state(&path).unwrap().unwrap();
        assert_eq!(config.iteration, 1);
        assert_eq!(config.last_issue_change_iteration, 0);
        assert!(config.issue_snapshot.is_empty());
    }

    #[test]
    fn test_write_session_state() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.yaml");

        let config = SessionConfig {
            iteration: 3,
            last_issue_change_iteration: 2,
            issue_snapshot: vec!["issue-1".to_string(), "issue-2".to_string()],
            ..Default::default()
        };

        write_session_state(&path, &config).unwrap();

        // Read it back
        let parsed = parse_session_state(&path).unwrap().unwrap();
        assert_eq!(parsed.iteration, 3);
        assert_eq!(parsed.issue_snapshot, vec!["issue-1", "issue-2"]);
    }

    #[test]
    fn test_cleanup_session_state() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.yaml");
        fs::write(&path, "content").unwrap();

        assert!(path.exists());
        cleanup_session_state(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn test_cleanup_session_state_not_exists() {
        let path = Path::new("/nonexistent/file.yaml");
        // Should not error
        cleanup_session_state(path).unwrap();
    }

    #[test]
    fn test_cleanup_session_files() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create both files
        let state_path = base.join(SESSION_STATE_PATH);
        let notes_path = base.join(SESSION_NOTES_PATH);
        fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        fs::write(&state_path, "state").unwrap();
        fs::write(&notes_path, "notes").unwrap();

        assert!(state_path.exists());
        assert!(notes_path.exists());

        cleanup_session_files(base).unwrap();

        assert!(!state_path.exists());
        assert!(!notes_path.exists());
    }

    #[test]
    fn test_session_config_iterations_since_change() {
        let config =
            SessionConfig { iteration: 10, last_issue_change_iteration: 7, ..Default::default() };
        assert_eq!(config.iterations_since_change(), 3);
    }

    #[test]
    fn test_session_config_is_stale() {
        let stale =
            SessionConfig { iteration: 10, last_issue_change_iteration: 4, ..Default::default() };
        assert!(stale.is_stale()); // 6 iterations since change

        let not_stale =
            SessionConfig { iteration: 10, last_issue_change_iteration: 8, ..Default::default() };
        assert!(!not_stale.is_stale()); // 2 iterations since change
    }

    #[test]
    fn test_session_config_issue_snapshot_set() {
        let config = SessionConfig {
            iteration: 1,
            last_issue_change_iteration: 1,
            issue_snapshot: vec!["a".to_string(), "b".to_string(), "a".to_string()],
            ..Default::default()
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
    fn test_write_session_state_creates_parent_directory() {
        let dir = TempDir::new().unwrap();
        // Path with non-existent parent directory
        let nested = dir.path().join("deeply").join("nested").join("path");
        let path = nested.join("state.yaml");

        // Verify parent doesn't exist yet
        assert!(!nested.exists());

        let config =
            SessionConfig { iteration: 1, last_issue_change_iteration: 1, ..Default::default() };

        write_session_state(&path, &config).unwrap();

        // Verify both parent and file now exist
        assert!(nested.exists());
        assert!(path.exists());
    }

    #[test]
    fn test_problem_mode_not_active_by_default() {
        let dir = TempDir::new().unwrap();
        assert!(!is_problem_mode_active(dir.path()));
    }

    #[test]
    fn test_enter_problem_mode() {
        let dir = TempDir::new().unwrap();

        // Enter problem mode
        enter_problem_mode(dir.path()).unwrap();

        // Verify marker file exists
        assert!(is_problem_mode_active(dir.path()));
        assert!(dir.path().join(PROBLEM_MODE_MARKER_PATH).exists());
    }

    #[test]
    fn test_exit_problem_mode() {
        let dir = TempDir::new().unwrap();

        // Enter and then exit problem mode
        enter_problem_mode(dir.path()).unwrap();
        assert!(is_problem_mode_active(dir.path()));

        exit_problem_mode(dir.path()).unwrap();
        assert!(!is_problem_mode_active(dir.path()));
    }

    #[test]
    fn test_exit_problem_mode_when_not_active() {
        let dir = TempDir::new().unwrap();

        // Should not error when exiting without entering
        exit_problem_mode(dir.path()).unwrap();
        assert!(!is_problem_mode_active(dir.path()));
    }

    #[test]
    fn test_enter_problem_mode_creates_parent_directory() {
        let dir = TempDir::new().unwrap();
        // Base dir is the temp dir, .claude subdirectory doesn't exist yet

        enter_problem_mode(dir.path()).unwrap();

        assert!(dir.path().join(".claude").exists());
        assert!(is_problem_mode_active(dir.path()));
    }
}
