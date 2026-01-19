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
}
