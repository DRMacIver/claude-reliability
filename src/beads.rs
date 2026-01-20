//! Beads integration for issue tracking.
//!
//! Beads is an optional issue tracking system. This module provides
//! integration that is only active when:
//! 1. The `bd` CLI is available
//! 2. A `.beads/` directory exists in the repository

use crate::error::Result;
use crate::traits::CommandRunner;
use std::collections::HashSet;
use std::path::Path;

/// Directory containing beads data.
pub const BEADS_DIR: &str = ".beads";

/// Marker file for tracking beads warning state (relative to base).
const WARNING_MARKER_REL: &str = ".claude/beads-warning-given.local";

/// Marker file for tracking beads warning state.
pub const BEADS_WARNING_MARKER: &str = ".claude/beads-warning-given.local";

/// Check if beads is available (CLI present and repo has .beads/).
pub fn is_beads_available(runner: &dyn CommandRunner) -> bool {
    is_beads_available_in(runner, Path::new("."))
}

/// Check if beads is available in the specified directory.
pub fn is_beads_available_in(runner: &dyn CommandRunner, base_dir: &Path) -> bool {
    // Check if bd CLI is available
    if !runner.is_available("bd") {
        return false;
    }

    // Check if .beads/ directory exists
    base_dir.join(BEADS_DIR).is_dir()
}

/// Status of agent's interaction with beads during this session.
#[derive(Debug, Clone, Default)]
pub struct BeadsInteractionStatus {
    /// True if .beads/ was modified (staged, unstaged, or commits).
    pub has_interaction: bool,
    /// True if we've already warned about missing interaction.
    pub already_warned: bool,
}

/// Check if the agent has interacted with beads during this session.
///
/// # Errors
///
/// Returns an error if git commands fail.
pub fn check_beads_interaction(runner: &dyn CommandRunner) -> Result<BeadsInteractionStatus> {
    check_beads_interaction_in(runner, Path::new("."))
}

/// Check if the agent has interacted with beads during this session in the specified directory.
///
/// # Errors
///
/// Returns an error if git commands fail.
pub fn check_beads_interaction_in(
    runner: &dyn CommandRunner,
    base_dir: &Path,
) -> Result<BeadsInteractionStatus> {
    let marker_path = base_dir.join(WARNING_MARKER_REL);
    let already_warned = marker_path.exists();

    // Check for uncommitted beads changes (staged or unstaged)
    let beads_diff = runner.run("git", &["diff", "--name-only", "--", ".beads/"], None)?;
    let beads_staged =
        runner.run("git", &["diff", "--cached", "--name-only", "--", ".beads/"], None)?;

    if !beads_diff.stdout.trim().is_empty() || !beads_staged.stdout.trim().is_empty() {
        // Beads was modified - clear any warning marker
        if marker_path.exists() {
            let _ = std::fs::remove_file(&marker_path);
        }
        return Ok(BeadsInteractionStatus { has_interaction: true, already_warned });
    }

    // Check recent commits for beads changes (last 10 commits)
    let recent_beads =
        runner.run("git", &["log", "--oneline", "-10", "--name-only", "--", ".beads/"], None)?;

    if !recent_beads.stdout.trim().is_empty() {
        // Recent beads activity found - clear any warning marker
        if marker_path.exists() {
            let _ = std::fs::remove_file(&marker_path);
        }
        return Ok(BeadsInteractionStatus { has_interaction: true, already_warned });
    }

    Ok(BeadsInteractionStatus { has_interaction: false, already_warned })
}

/// Mark that a beads warning has been given.
///
/// # Errors
///
/// Returns an error if the marker file cannot be written.
pub fn mark_beads_warning_given() -> Result<()> {
    mark_beads_warning_given_in(Path::new("."))
}

/// Mark that a beads warning has been given in the specified directory.
///
/// # Errors
///
/// Returns an error if the marker file cannot be written.
pub fn mark_beads_warning_given_in(base_dir: &Path) -> Result<()> {
    let marker_path = base_dir.join(WARNING_MARKER_REL);
    // Ensure parent directory exists
    if let Some(parent) = marker_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(marker_path, "")?;
    Ok(())
}

/// Clear the beads warning marker.
///
/// # Errors
///
/// Returns an error if the marker file cannot be removed.
pub fn clear_beads_warning() -> Result<()> {
    clear_beads_warning_in(Path::new("."))
}

/// Clear the beads warning marker in the specified directory.
///
/// # Errors
///
/// Returns an error if the marker file cannot be removed.
pub fn clear_beads_warning_in(base_dir: &Path) -> Result<()> {
    let marker_path = base_dir.join(WARNING_MARKER_REL);
    if marker_path.exists() {
        std::fs::remove_file(marker_path)?;
    }
    Ok(())
}

/// Get the count of open issues.
///
/// # Errors
///
/// Returns an error if bd commands fail.
#[allow(clippy::cast_possible_truncation)] // Issue counts won't exceed u32::MAX
pub fn get_open_issues_count(runner: &dyn CommandRunner) -> Result<u32> {
    let output = runner.run("bd", &["list", "--status=open", "--format=json"], None)?;

    if !output.success() {
        return Ok(0);
    }

    // Try to parse JSON output
    if let Ok(issues) = serde_json::from_str::<Vec<serde_json::Value>>(&output.stdout) {
        return Ok(issues.len() as u32);
    }

    // Fallback: count lines from bd ready
    let ready_output = runner.run("bd", &["ready"], None)?;
    if ready_output.success() {
        let count = ready_output
            .stdout
            .lines()
            .filter(|line| {
                let line = line.trim();
                !line.is_empty()
                    && line.chars().next().is_some_and(|c| c.is_ascii_digit())
                    && line.contains('[')
            })
            .count();
        return Ok(count as u32);
    }

    Ok(0)
}

/// Get the count of issues that are ready to work on (no blockers).
///
/// Uses `bd ready` which only shows issues without blockers.
///
/// # Errors
///
/// Returns an error if bd commands fail.
#[allow(clippy::cast_possible_truncation)] // Issue counts won't exceed u32::MAX
pub fn get_ready_issues_count(runner: &dyn CommandRunner) -> Result<u32> {
    let output = runner.run("bd", &["ready"], None)?;

    if !output.success() {
        return Ok(0);
    }

    // Count lines that look like issue entries (start with digit and contain '[')
    let count = output
        .stdout
        .lines()
        .filter(|line| {
            let line = line.trim();
            !line.is_empty()
                && line.chars().next().is_some_and(|c| c.is_ascii_digit())
                && line.contains('[')
        })
        .count();

    Ok(count as u32)
}

/// Get current open and in-progress issue IDs.
///
/// # Errors
///
/// Returns an error if bd commands fail.
pub fn get_current_issues(
    runner: &dyn CommandRunner,
) -> Result<(HashSet<String>, HashSet<String>)> {
    let open_output = runner.run("bd", &["list", "--status=open"], None)?;
    let in_progress_output = runner.run("bd", &["list", "--status=in_progress"], None)?;

    let open_ids = extract_issue_ids(&open_output.stdout);
    let in_progress_ids = extract_issue_ids(&in_progress_output.stdout);

    Ok((open_ids, in_progress_ids))
}

/// Extract issue IDs from bd output.
fn extract_issue_ids(output: &str) -> HashSet<String> {
    let mut ids = HashSet::new();
    for line in output.lines() {
        // Look for issue ID patterns (e.g., "project-123")
        for word in line.split_whitespace() {
            if word.contains('-') && word.chars().any(|c| c.is_ascii_digit()) {
                // Take the first word that looks like an issue ID
                ids.insert(word.to_string());
                break;
            }
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::MockCommandRunner;
    use crate::traits::CommandOutput;
    use tempfile::TempDir;

    #[test]
    fn test_is_beads_available_no_cli() {
        let runner = MockCommandRunner::new();
        // CLI not available
        assert!(!is_beads_available(&runner));
    }

    #[test]
    fn test_extract_issue_ids() {
        let output = r"○ project-123 [P2] [bug] - Fix something
● project-456 [P1] [feature] - Add something
";
        let ids = extract_issue_ids(output);
        assert_eq!(ids.len(), 2);
        assert!(ids.contains("project-123"));
        assert!(ids.contains("project-456"));
    }

    #[test]
    fn test_extract_issue_ids_empty() {
        let ids = extract_issue_ids("");
        assert!(ids.is_empty());
    }

    #[test]
    fn test_extract_issue_ids_no_ids() {
        let ids = extract_issue_ids("No issues found\nAnother line");
        assert!(ids.is_empty());
    }

    #[test]
    fn test_get_open_issues_count_json() {
        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");
        runner.expect(
            "bd",
            &["list", "--status=open", "--format=json"],
            CommandOutput {
                exit_code: 0,
                stdout: r#"[{"id": "a"}, {"id": "b"}, {"id": "c"}]"#.to_string(),
                stderr: String::new(),
            },
        );

        let count = get_open_issues_count(&runner).unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    fn test_get_open_issues_count_fallback() {
        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");
        // First command succeeds but returns invalid JSON
        runner.expect(
            "bd",
            &["list", "--status=open", "--format=json"],
            CommandOutput {
                exit_code: 0,
                stdout: "not valid json".to_string(),
                stderr: String::new(),
            },
        );
        // Fallback to bd ready
        runner.expect(
            "bd",
            &["ready"],
            CommandOutput {
                exit_code: 0,
                stdout: "1 [P1] Issue one\n2 [P2] Issue two\n".to_string(),
                stderr: String::new(),
            },
        );

        let count = get_open_issues_count(&runner).unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn test_beads_warning_marker() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Marker doesn't exist initially
        assert!(!base.join(WARNING_MARKER_REL).exists());

        // Create the .claude directory and marker
        mark_beads_warning_given_in(base).unwrap();
        assert!(base.join(WARNING_MARKER_REL).exists());

        // Clear it
        clear_beads_warning_in(base).unwrap();
        assert!(!base.join(WARNING_MARKER_REL).exists());
    }

    #[test]
    fn test_is_beads_available_cli_but_no_dir() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");
        // CLI is available but no .beads/ directory
        assert!(!is_beads_available_in(&runner, base));
    }

    #[test]
    fn test_is_beads_available_cli_and_dir() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create .beads/ directory
        std::fs::create_dir_all(base.join(BEADS_DIR)).unwrap();

        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");
        assert!(is_beads_available_in(&runner, base));
    }

    #[test]
    fn test_get_open_issues_count_command_fails() {
        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");
        runner.expect(
            "bd",
            &["list", "--status=open", "--format=json"],
            CommandOutput { exit_code: 1, stdout: String::new(), stderr: "error".to_string() },
        );

        let count = get_open_issues_count(&runner).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_get_open_issues_count_fallback_fails() {
        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");
        // JSON parse fails
        runner.expect(
            "bd",
            &["list", "--status=open", "--format=json"],
            CommandOutput { exit_code: 0, stdout: "not json".to_string(), stderr: String::new() },
        );
        // Fallback also fails
        runner.expect(
            "bd",
            &["ready"],
            CommandOutput { exit_code: 1, stdout: String::new(), stderr: "error".to_string() },
        );

        let count = get_open_issues_count(&runner).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_get_ready_issues_count() {
        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");
        runner.expect(
            "bd",
            &["ready"],
            CommandOutput {
                exit_code: 0,
                stdout: "1 [P1] Ready issue one\n2 [P2] Ready issue two\n".to_string(),
                stderr: String::new(),
            },
        );

        let count = get_ready_issues_count(&runner).unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn test_get_ready_issues_count_no_issues() {
        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");
        runner.expect(
            "bd",
            &["ready"],
            CommandOutput {
                exit_code: 0,
                stdout: "No ready issues\n".to_string(),
                stderr: String::new(),
            },
        );

        let count = get_ready_issues_count(&runner).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_get_ready_issues_count_command_fails() {
        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");
        runner.expect(
            "bd",
            &["ready"],
            CommandOutput { exit_code: 1, stdout: String::new(), stderr: "error".to_string() },
        );

        let count = get_ready_issues_count(&runner).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_get_current_issues() {
        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");
        runner.expect(
            "bd",
            &["list", "--status=open"],
            CommandOutput {
                exit_code: 0,
                stdout: "project-1 [P1] Open issue\nproject-2 [P2] Another\n".to_string(),
                stderr: String::new(),
            },
        );
        runner.expect(
            "bd",
            &["list", "--status=in_progress"],
            CommandOutput {
                exit_code: 0,
                stdout: "project-3 [P1] In progress\n".to_string(),
                stderr: String::new(),
            },
        );

        let (open, in_progress) = get_current_issues(&runner).unwrap();
        assert_eq!(open.len(), 2);
        assert_eq!(in_progress.len(), 1);
        assert!(open.contains("project-1"));
        assert!(open.contains("project-2"));
        assert!(in_progress.contains("project-3"));
    }

    #[test]
    fn test_check_beads_interaction_uncommitted() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let mut runner = MockCommandRunner::new();
        // Both commands are called unconditionally
        runner.expect(
            "git",
            &["diff", "--name-only", "--", ".beads/"],
            CommandOutput {
                exit_code: 0,
                stdout: ".beads/issues.yml\n".to_string(),
                stderr: String::new(),
            },
        );
        runner.expect(
            "git",
            &["diff", "--cached", "--name-only", "--", ".beads/"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );

        let status = check_beads_interaction_in(&runner, base).unwrap();
        assert!(status.has_interaction);
    }

    #[test]
    fn test_check_beads_interaction_staged() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let mut runner = MockCommandRunner::new();
        // No uncommitted changes
        runner.expect(
            "git",
            &["diff", "--name-only", "--", ".beads/"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        // Staged changes exist
        runner.expect(
            "git",
            &["diff", "--cached", "--name-only", "--", ".beads/"],
            CommandOutput {
                exit_code: 0,
                stdout: ".beads/issues.yml\n".to_string(),
                stderr: String::new(),
            },
        );

        let status = check_beads_interaction_in(&runner, base).unwrap();
        assert!(status.has_interaction);
    }

    #[test]
    fn test_check_beads_interaction_recent_commits() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let mut runner = MockCommandRunner::new();
        // No uncommitted changes
        runner.expect(
            "git",
            &["diff", "--name-only", "--", ".beads/"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        // No staged changes
        runner.expect(
            "git",
            &["diff", "--cached", "--name-only", "--", ".beads/"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        // Recent commits have beads changes
        runner.expect(
            "git",
            &["log", "--oneline", "-10", "--name-only", "--", ".beads/"],
            CommandOutput {
                exit_code: 0,
                stdout: "abc123 commit\n.beads/issues.yml\n".to_string(),
                stderr: String::new(),
            },
        );

        let status = check_beads_interaction_in(&runner, base).unwrap();
        assert!(status.has_interaction);
    }

    #[test]
    fn test_check_beads_interaction_none() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let mut runner = MockCommandRunner::new();
        // No uncommitted changes
        runner.expect(
            "git",
            &["diff", "--name-only", "--", ".beads/"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        // No staged changes
        runner.expect(
            "git",
            &["diff", "--cached", "--name-only", "--", ".beads/"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        // No recent commits with beads
        runner.expect(
            "git",
            &["log", "--oneline", "-10", "--name-only", "--", ".beads/"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );

        let status = check_beads_interaction_in(&runner, base).unwrap();
        assert!(!status.has_interaction);
    }

    #[test]
    fn test_check_beads_interaction_already_warned() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create warning marker
        mark_beads_warning_given_in(base).unwrap();

        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--name-only", "--", ".beads/"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["diff", "--cached", "--name-only", "--", ".beads/"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["log", "--oneline", "-10", "--name-only", "--", ".beads/"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );

        let status = check_beads_interaction_in(&runner, base).unwrap();
        assert!(!status.has_interaction);
        assert!(status.already_warned);
    }

    #[test]
    fn test_clear_beads_warning_not_exists() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Should not error when marker doesn't exist
        clear_beads_warning_in(base).unwrap();
    }

    #[test]
    fn test_check_beads_interaction_uncommitted_clears_warning() {
        // Test that uncommitted beads changes clear the warning marker (line 76)
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create warning marker first
        mark_beads_warning_given_in(base).unwrap();
        assert!(base.join(WARNING_MARKER_REL).exists());

        let mut runner = MockCommandRunner::new();
        // Uncommitted beads changes exist
        runner.expect(
            "git",
            &["diff", "--name-only", "--", ".beads/"],
            CommandOutput {
                exit_code: 0,
                stdout: ".beads/issues.yml\n".to_string(),
                stderr: String::new(),
            },
        );
        runner.expect(
            "git",
            &["diff", "--cached", "--name-only", "--", ".beads/"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );

        let status = check_beads_interaction_in(&runner, base).unwrap();
        assert!(status.has_interaction);
        assert!(status.already_warned); // Was true before the check

        // Warning marker should be cleared now
        assert!(!base.join(WARNING_MARKER_REL).exists());
    }

    #[test]
    fn test_check_beads_interaction_recent_commits_clears_warning() {
        // Test that recent beads commits clear the warning marker (line 88)
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create warning marker first
        mark_beads_warning_given_in(base).unwrap();
        assert!(base.join(WARNING_MARKER_REL).exists());

        let mut runner = MockCommandRunner::new();
        // No uncommitted changes
        runner.expect(
            "git",
            &["diff", "--name-only", "--", ".beads/"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["diff", "--cached", "--name-only", "--", ".beads/"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        // Recent commits with beads changes
        runner.expect(
            "git",
            &["log", "--oneline", "-10", "--name-only", "--", ".beads/"],
            CommandOutput {
                exit_code: 0,
                stdout: "abc123 Updated issue\n.beads/issues.yml\n".to_string(),
                stderr: String::new(),
            },
        );

        let status = check_beads_interaction_in(&runner, base).unwrap();
        assert!(status.has_interaction);
        assert!(status.already_warned);

        // Warning marker should be cleared now
        assert!(!base.join(WARNING_MARKER_REL).exists());
    }

    // Tests for the wrapper functions that use the current working directory.
    // These tests depend on the actual workspace having a .beads directory.

    #[test]
    fn test_is_beads_available_wrapper() {
        // This test uses the actual workspace which has a .beads directory
        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");
        // The wrapper calls is_beads_available_in with Path::new(".")
        // Since we're in the workspace with .beads, this should return true
        assert!(is_beads_available(&runner));
    }

    #[test]
    fn test_check_beads_interaction_wrapper() {
        // This test uses the actual workspace
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--name-only", "--", ".beads/"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["diff", "--cached", "--name-only", "--", ".beads/"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["log", "--oneline", "-10", "--name-only", "--", ".beads/"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );

        // Uses the current directory (workspace)
        let status = check_beads_interaction(&runner).unwrap();
        // No interaction, not already warned (unless marker exists from previous test)
        assert!(!status.has_interaction);
    }

    #[test]
    fn test_mark_and_clear_beads_warning_wrapper() {
        // This test uses the actual workspace
        // First clear any existing warning
        let _ = clear_beads_warning();

        // Mark warning
        mark_beads_warning_given().unwrap();
        assert!(std::path::Path::new(WARNING_MARKER_REL).exists());

        // Clear warning
        clear_beads_warning().unwrap();
        assert!(!std::path::Path::new(WARNING_MARKER_REL).exists());
    }
}
