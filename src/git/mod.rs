//! Git operations module.

mod diff;
mod status;

pub use diff::{parse_diff, AddedLine, DiffHunk};
pub use status::{check_uncommitted_changes, GitStatus, UncommittedChanges};

use crate::error::Result;
use crate::traits::CommandRunner;

/// Check if we're in a git repository.
pub fn is_git_repo(runner: &dyn CommandRunner) -> bool {
    runner.run("git", &["rev-parse", "--git-dir"], None).map(|o| o.success()).unwrap_or(false)
}

/// Get the current branch name.
///
/// # Errors
///
/// Returns an error if the git command fails.
pub fn current_branch(runner: &dyn CommandRunner) -> Result<Option<String>> {
    let output = runner.run("git", &["rev-parse", "--abbrev-ref", "HEAD"], None)?;
    if output.success() {
        let branch = output.stdout.trim();
        if branch.is_empty() || branch == "HEAD" {
            Ok(None)
        } else {
            Ok(Some(branch.to_string()))
        }
    } else {
        Ok(None)
    }
}

/// Get the current commit SHA.
///
/// # Errors
///
/// Returns an error if the git command fails.
pub fn current_sha(runner: &dyn CommandRunner) -> Result<Option<String>> {
    let output = runner.run("git", &["rev-parse", "HEAD"], None)?;
    if output.success() {
        let sha = output.stdout.trim();
        if sha.is_empty() {
            Ok(None)
        } else {
            Ok(Some(sha.to_string()))
        }
    } else {
        Ok(None)
    }
}

/// Get staged files.
///
/// # Errors
///
/// Returns an error if the git command fails.
pub fn staged_files(runner: &dyn CommandRunner) -> Result<Vec<String>> {
    let output = runner.run("git", &["diff", "--cached", "--name-only"], None)?;
    if output.success() {
        Ok(output.stdout.lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
    } else {
        Ok(Vec::new())
    }
}

/// Get the staged diff.
///
/// # Errors
///
/// Returns an error if the git command fails.
pub fn staged_diff(runner: &dyn CommandRunner) -> Result<String> {
    let output = runner.run("git", &["diff", "--cached", "-U0"], None)?;
    Ok(output.stdout)
}

/// Get the unstaged diff.
///
/// # Errors
///
/// Returns an error if the git command fails.
pub fn unstaged_diff(runner: &dyn CommandRunner) -> Result<String> {
    let output = runner.run("git", &["diff", "-U0"], None)?;
    Ok(output.stdout)
}

/// Get combined staged and unstaged diff.
///
/// # Errors
///
/// Returns an error if the git commands fail.
pub fn combined_diff(runner: &dyn CommandRunner) -> Result<String> {
    let staged = staged_diff(runner)?;
    let unstaged = unstaged_diff(runner)?;
    if staged.is_empty() {
        Ok(unstaged)
    } else if unstaged.is_empty() {
        Ok(staged)
    } else {
        Ok(format!("{staged}\n{unstaged}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::MockCommandRunner;
    use crate::traits::CommandOutput;

    #[test]
    fn test_is_git_repo_true() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["rev-parse", "--git-dir"],
            CommandOutput { exit_code: 0, stdout: ".git\n".to_string(), stderr: String::new() },
        );
        assert!(is_git_repo(&runner));
    }

    #[test]
    fn test_is_git_repo_false() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["rev-parse", "--git-dir"],
            CommandOutput {
                exit_code: 128,
                stdout: String::new(),
                stderr: "fatal: not a git repository\n".to_string(),
            },
        );
        assert!(!is_git_repo(&runner));
    }

    #[test]
    fn test_current_branch() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["rev-parse", "--abbrev-ref", "HEAD"],
            CommandOutput { exit_code: 0, stdout: "main\n".to_string(), stderr: String::new() },
        );
        assert_eq!(current_branch(&runner).unwrap(), Some("main".to_string()));
    }

    #[test]
    fn test_staged_files() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--cached", "--name-only"],
            CommandOutput {
                exit_code: 0,
                stdout: "src/lib.rs\nsrc/main.rs\n".to_string(),
                stderr: String::new(),
            },
        );
        let files = staged_files(&runner).unwrap();
        assert_eq!(files, vec!["src/lib.rs", "src/main.rs"]);
    }

    #[test]
    fn test_staged_files_empty() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--cached", "--name-only"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        let files = staged_files(&runner).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn test_staged_files_failure() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--cached", "--name-only"],
            CommandOutput { exit_code: 1, stdout: String::new(), stderr: "error".to_string() },
        );
        let files = staged_files(&runner).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn test_current_branch_failure() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["rev-parse", "--abbrev-ref", "HEAD"],
            CommandOutput { exit_code: 128, stdout: String::new(), stderr: "error".to_string() },
        );
        assert_eq!(current_branch(&runner).unwrap(), None);
    }

    #[test]
    fn test_current_branch_detached_head() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["rev-parse", "--abbrev-ref", "HEAD"],
            CommandOutput { exit_code: 0, stdout: "HEAD\n".to_string(), stderr: String::new() },
        );
        assert_eq!(current_branch(&runner).unwrap(), None);
    }

    #[test]
    fn test_current_sha() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["rev-parse", "HEAD"],
            CommandOutput {
                exit_code: 0,
                stdout: "abc123def456\n".to_string(),
                stderr: String::new(),
            },
        );
        assert_eq!(current_sha(&runner).unwrap(), Some("abc123def456".to_string()));
    }

    #[test]
    fn test_current_sha_failure() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["rev-parse", "HEAD"],
            CommandOutput { exit_code: 128, stdout: String::new(), stderr: "error".to_string() },
        );
        assert_eq!(current_sha(&runner).unwrap(), None);
    }

    #[test]
    fn test_current_sha_empty() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["rev-parse", "HEAD"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        assert_eq!(current_sha(&runner).unwrap(), None);
    }

    #[test]
    fn test_staged_diff() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--cached", "-U0"],
            CommandOutput {
                exit_code: 0,
                stdout: "+some change\n".to_string(),
                stderr: String::new(),
            },
        );
        let diff = staged_diff(&runner).unwrap();
        assert_eq!(diff, "+some change\n");
    }

    #[test]
    fn test_unstaged_diff() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "-U0"],
            CommandOutput {
                exit_code: 0,
                stdout: "+unstaged change\n".to_string(),
                stderr: String::new(),
            },
        );
        let diff = unstaged_diff(&runner).unwrap();
        assert_eq!(diff, "+unstaged change\n");
    }

    #[test]
    fn test_combined_diff_both() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--cached", "-U0"],
            CommandOutput { exit_code: 0, stdout: "staged\n".to_string(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["diff", "-U0"],
            CommandOutput { exit_code: 0, stdout: "unstaged\n".to_string(), stderr: String::new() },
        );
        let diff = combined_diff(&runner).unwrap();
        assert_eq!(diff, "staged\n\nunstaged\n");
    }

    #[test]
    fn test_combined_diff_only_staged() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--cached", "-U0"],
            CommandOutput { exit_code: 0, stdout: "staged\n".to_string(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["diff", "-U0"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        let diff = combined_diff(&runner).unwrap();
        assert_eq!(diff, "staged\n");
    }

    #[test]
    fn test_combined_diff_only_unstaged() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--cached", "-U0"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["diff", "-U0"],
            CommandOutput { exit_code: 0, stdout: "unstaged\n".to_string(), stderr: String::new() },
        );
        let diff = combined_diff(&runner).unwrap();
        assert_eq!(diff, "unstaged\n");
    }
}
