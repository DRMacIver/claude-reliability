//! Git status checking.

use crate::error::Result;
use crate::traits::CommandRunner;

/// Represents uncommitted changes in a git repository.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UncommittedChanges {
    /// Whether there are unstaged changes.
    pub has_unstaged: bool,
    /// Whether there are staged changes.
    pub has_staged: bool,
    /// Whether there are untracked files.
    pub has_untracked: bool,
}

impl UncommittedChanges {
    /// Check if there are any uncommitted changes.
    #[must_use]
    pub const fn has_changes(&self) -> bool {
        self.has_unstaged || self.has_staged || self.has_untracked
    }

    /// Get a human-readable description of the changes.
    #[must_use]
    pub fn description(&self) -> String {
        let mut parts = Vec::new();
        if self.has_unstaged {
            parts.push("unstaged changes");
        }
        if self.has_staged {
            parts.push("staged changes");
        }
        if self.has_untracked {
            parts.push("untracked files");
        }
        parts.join(", ")
    }
}

/// Full git status information.
#[derive(Debug, Clone, Default)]
pub struct GitStatus {
    /// Uncommitted changes.
    pub uncommitted: UncommittedChanges,
    /// List of untracked files.
    pub untracked_files: Vec<String>,
    /// Whether the branch is ahead of the remote.
    pub ahead_of_remote: bool,
    /// Number of commits ahead.
    pub commits_ahead: u32,
}

/// Check for uncommitted changes in the git repository.
///
/// # Errors
///
/// Returns an error if git commands fail.
pub fn check_uncommitted_changes(runner: &dyn CommandRunner) -> Result<GitStatus> {
    let mut status = GitStatus::default();

    // Check for unstaged changes
    let diff_output = runner.run("git", &["diff", "--stat"], None)?;
    status.uncommitted.has_unstaged =
        diff_output.success() && !diff_output.stdout.trim().is_empty();

    // Check for staged changes
    let staged_output = runner.run("git", &["diff", "--cached", "--stat"], None)?;
    status.uncommitted.has_staged =
        staged_output.success() && !staged_output.stdout.trim().is_empty();

    // Check for untracked files
    let untracked_output =
        runner.run("git", &["ls-files", "--others", "--exclude-standard"], None)?;
    if untracked_output.success() {
        let files: Vec<String> = untracked_output
            .stdout
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        status.uncommitted.has_untracked = !files.is_empty();
        status.untracked_files = files;
    }

    // Check if ahead of remote
    let rev_list = runner.run("git", &["rev-list", "--count", "@{upstream}..HEAD"], None);
    if let Ok(output) = rev_list {
        if output.success() {
            if let Ok(count) = output.stdout.trim().parse::<u32>() {
                status.ahead_of_remote = count > 0;
                status.commits_ahead = count;
            }
        }
    }

    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::MockCommandRunner;
    use crate::traits::CommandOutput;

    #[test]
    fn test_uncommitted_changes_has_changes() {
        let changes =
            UncommittedChanges { has_unstaged: true, has_staged: false, has_untracked: false };
        assert!(changes.has_changes());

        let no_changes = UncommittedChanges::default();
        assert!(!no_changes.has_changes());
    }

    #[test]
    fn test_uncommitted_changes_description() {
        let changes =
            UncommittedChanges { has_unstaged: true, has_staged: true, has_untracked: false };
        assert_eq!(changes.description(), "unstaged changes, staged changes");

        let all_changes =
            UncommittedChanges { has_unstaged: true, has_staged: true, has_untracked: true };
        assert_eq!(all_changes.description(), "unstaged changes, staged changes, untracked files");
    }

    #[test]
    fn test_check_uncommitted_changes_clean() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--stat"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["diff", "--cached", "--stat"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["rev-list", "--count", "@{upstream}..HEAD"],
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() },
        );

        let status = check_uncommitted_changes(&runner).unwrap();
        assert!(!status.uncommitted.has_changes());
    }

    #[test]
    fn test_check_uncommitted_changes_with_unstaged() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--stat"],
            CommandOutput {
                exit_code: 0,
                stdout: " src/lib.rs | 5 +++++\n 1 file changed\n".to_string(),
                stderr: String::new(),
            },
        );
        runner.expect(
            "git",
            &["diff", "--cached", "--stat"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["rev-list", "--count", "@{upstream}..HEAD"],
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() },
        );

        let status = check_uncommitted_changes(&runner).unwrap();
        assert!(status.uncommitted.has_unstaged);
        assert!(!status.uncommitted.has_staged);
        assert!(!status.uncommitted.has_untracked);
    }

    #[test]
    fn test_check_uncommitted_changes_with_untracked() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--stat"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["diff", "--cached", "--stat"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            CommandOutput {
                exit_code: 0,
                stdout: "new_file.rs\nanother_file.txt\n".to_string(),
                stderr: String::new(),
            },
        );
        runner.expect(
            "git",
            &["rev-list", "--count", "@{upstream}..HEAD"],
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() },
        );

        let status = check_uncommitted_changes(&runner).unwrap();
        assert!(!status.uncommitted.has_unstaged);
        assert!(!status.uncommitted.has_staged);
        assert!(status.uncommitted.has_untracked);
        assert_eq!(status.untracked_files, vec!["new_file.rs", "another_file.txt"]);
    }

    #[test]
    fn test_check_uncommitted_changes_untracked_command_fails() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--stat"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["diff", "--cached", "--stat"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        // ls-files command fails
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            CommandOutput { exit_code: 1, stdout: String::new(), stderr: "error".to_string() },
        );
        runner.expect(
            "git",
            &["rev-list", "--count", "@{upstream}..HEAD"],
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() },
        );

        let status = check_uncommitted_changes(&runner).unwrap();
        // Untracked check is skipped when command fails
        assert!(!status.uncommitted.has_untracked);
        assert!(status.untracked_files.is_empty());
    }

    #[test]
    fn test_check_uncommitted_changes_rev_list_non_zero_exit() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--stat"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["diff", "--cached", "--stat"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        // rev-list command returns non-zero (e.g., no upstream set)
        runner.expect(
            "git",
            &["rev-list", "--count", "@{upstream}..HEAD"],
            CommandOutput {
                exit_code: 128,
                stdout: String::new(),
                stderr: "fatal: no upstream".to_string(),
            },
        );

        let status = check_uncommitted_changes(&runner).unwrap();
        // ahead_of_remote remains false when command fails
        assert!(!status.ahead_of_remote);
        assert_eq!(status.commits_ahead, 0);
    }

    #[test]
    fn test_check_uncommitted_changes_rev_list_invalid_output() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--stat"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["diff", "--cached", "--stat"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        // rev-list returns success but with unparseable output
        runner.expect(
            "git",
            &["rev-list", "--count", "@{upstream}..HEAD"],
            CommandOutput {
                exit_code: 0,
                stdout: "not a number\n".to_string(),
                stderr: String::new(),
            },
        );

        let status = check_uncommitted_changes(&runner).unwrap();
        // ahead_of_remote remains false when output can't be parsed
        assert!(!status.ahead_of_remote);
        assert_eq!(status.commits_ahead, 0);
    }
}
