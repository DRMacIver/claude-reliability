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

/// Marker file for tracking beads warning state.
pub const BEADS_WARNING_MARKER: &str = ".claude/beads-warning-given.local";

/// Check if beads is available (CLI present and repo has .beads/).
pub fn is_beads_available(runner: &dyn CommandRunner) -> bool {
    // Check if bd CLI is available
    if !runner.is_available("bd") {
        return false;
    }

    // Check if .beads/ directory exists
    Path::new(".beads").is_dir()
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
    let already_warned = Path::new(BEADS_WARNING_MARKER).exists();

    // Check for uncommitted beads changes (staged or unstaged)
    let beads_diff = runner.run("git", &["diff", "--name-only", "--", ".beads/"], None)?;
    let beads_staged =
        runner.run("git", &["diff", "--cached", "--name-only", "--", ".beads/"], None)?;

    if !beads_diff.stdout.trim().is_empty() || !beads_staged.stdout.trim().is_empty() {
        // Beads was modified - clear any warning marker
        if Path::new(BEADS_WARNING_MARKER).exists() {
            let _ = std::fs::remove_file(BEADS_WARNING_MARKER);
        }
        return Ok(BeadsInteractionStatus { has_interaction: true, already_warned });
    }

    // Check recent commits for beads changes (last 10 commits)
    let recent_beads =
        runner.run("git", &["log", "--oneline", "-10", "--name-only", "--", ".beads/"], None)?;

    if !recent_beads.stdout.trim().is_empty() {
        // Recent beads activity found - clear any warning marker
        if Path::new(BEADS_WARNING_MARKER).exists() {
            let _ = std::fs::remove_file(BEADS_WARNING_MARKER);
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
    // Ensure parent directory exists
    if let Some(parent) = Path::new(BEADS_WARNING_MARKER).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(BEADS_WARNING_MARKER, "")?;
    Ok(())
}

/// Clear the beads warning marker.
///
/// # Errors
///
/// Returns an error if the marker file cannot be removed.
pub fn clear_beads_warning() -> Result<()> {
    if Path::new(BEADS_WARNING_MARKER).exists() {
        std::fs::remove_file(BEADS_WARNING_MARKER)?;
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
        std::env::set_current_dir(&dir).unwrap();

        // Marker doesn't exist initially
        assert!(!Path::new(BEADS_WARNING_MARKER).exists());

        // Create the .claude directory and marker
        std::fs::create_dir_all(".claude").unwrap();
        mark_beads_warning_given().unwrap();
        assert!(Path::new(BEADS_WARNING_MARKER).exists());

        // Clear it
        clear_beads_warning().unwrap();
        assert!(!Path::new(BEADS_WARNING_MARKER).exists());
    }
}
