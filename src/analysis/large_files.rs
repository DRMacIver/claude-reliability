//! Large file detection.

use crate::analysis::Violation;
use crate::traits::CommandRunner;
use std::path::Path;

/// Threshold for large files (500KB).
pub const LARGE_FILE_THRESHOLD_BYTES: u64 = 500 * 1024;

/// Information about a large file.
#[derive(Debug, Clone)]
pub struct LargeFile {
    /// The file path.
    pub path: String,
    /// The file size in bytes.
    pub size_bytes: u64,
}

impl LargeFile {
    /// Format the size for display.
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // Acceptable for display of file sizes
    pub fn formatted_size(&self) -> String {
        let kb = self.size_bytes as f64 / 1024.0;
        let mb = kb / 1024.0;
        if mb >= 1.0 {
            format!("{mb:.1}MB")
        } else {
            format!("{kb:.0}KB")
        }
    }

    /// Convert to a violation.
    #[must_use]
    pub fn to_violation(&self) -> Violation {
        Violation::new(&self.path, 0, format!("large file ({})", self.formatted_size()))
    }
}

/// Check for large files in staged or untracked files.
///
/// # Arguments
///
/// * `runner` - Command runner for git operations.
///
/// # Returns
///
/// A list of violations for files exceeding the size threshold.
pub fn check_large_files(runner: &dyn CommandRunner) -> Vec<Violation> {
    let mut violations = Vec::new();

    // Get staged files
    let staged = runner
        .run("git", &["diff", "--cached", "--name-only"], None)
        .ok()
        .filter(super::super::traits::CommandOutput::success)
        .map(|o| o.stdout)
        .unwrap_or_default();

    // Get untracked files
    let untracked = runner
        .run("git", &["ls-files", "--others", "--exclude-standard"], None)
        .ok()
        .filter(super::super::traits::CommandOutput::success)
        .map(|o| o.stdout)
        .unwrap_or_default();

    // Combine and check sizes
    for filepath in staged.lines().chain(untracked.lines()) {
        let filepath = filepath.trim();
        if filepath.is_empty() {
            continue;
        }

        let path = Path::new(filepath);
        if let Ok(metadata) = std::fs::metadata(path) {
            if metadata.is_file() && metadata.len() > LARGE_FILE_THRESHOLD_BYTES {
                let large_file =
                    LargeFile { path: filepath.to_string(), size_bytes: metadata.len() };
                violations.push(large_file.to_violation());
            }
        }
    }

    violations
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::MockCommandRunner;
    use crate::traits::CommandOutput;

    #[test]
    fn test_check_large_files_no_files() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--cached", "--name-only"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );

        let violations = check_large_files(&runner);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_check_large_files_git_failure() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--cached", "--name-only"],
            CommandOutput { exit_code: 1, stdout: String::new(), stderr: "error".to_string() },
        );
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            CommandOutput { exit_code: 1, stdout: String::new(), stderr: "error".to_string() },
        );

        let violations = check_large_files(&runner);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_check_large_files_nonexistent_file() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--cached", "--name-only"],
            CommandOutput {
                exit_code: 0,
                stdout: "nonexistent_file.txt\n".to_string(),
                stderr: String::new(),
            },
        );
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );

        let violations = check_large_files(&runner);
        // File doesn't exist, so no violation
        assert!(violations.is_empty());
    }

    #[test]
    fn test_check_large_files_empty_lines() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--cached", "--name-only"],
            CommandOutput { exit_code: 0, stdout: "\n\n  \n".to_string(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );

        let violations = check_large_files(&runner);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_formatted_size_kb() {
        let file = LargeFile {
            path: "test.bin".to_string(),
            size_bytes: 512 * 1024, // 512KB
        };
        assert_eq!(file.formatted_size(), "512KB");
    }

    #[test]
    fn test_formatted_size_mb() {
        let file = LargeFile {
            path: "test.bin".to_string(),
            size_bytes: 2 * 1024 * 1024, // 2MB
        };
        assert_eq!(file.formatted_size(), "2.0MB");
    }

    #[test]
    fn test_to_violation() {
        let file = LargeFile { path: "big.bin".to_string(), size_bytes: 1024 * 1024 };
        let v = file.to_violation();
        assert_eq!(v.file, "big.bin");
        assert!(v.description.contains("large file"));
        assert!(v.description.contains("1.0MB"));
    }

    #[test]
    fn test_threshold_constant() {
        // Verify the threshold is 500KB
        assert_eq!(LARGE_FILE_THRESHOLD_BYTES, 500 * 1024);
    }

    #[test]
    fn test_check_large_files_detects_large_file() {
        use tempfile::TempDir;

        // Create a temp directory with a large file
        let dir = TempDir::new().unwrap();
        let large_file_path = dir.path().join("large.bin");

        // Create a file larger than 500KB threshold
        let large_content = vec![0u8; 600 * 1024]; // 600KB
        std::fs::write(&large_file_path, large_content).unwrap();

        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--cached", "--name-only"],
            CommandOutput {
                exit_code: 0,
                stdout: format!("{}\n", large_file_path.display()),
                stderr: String::new(),
            },
        );
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );

        let violations = check_large_files(&runner);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].description.contains("large file"));
    }

    #[test]
    fn test_check_large_files_small_file_ignored() {
        use tempfile::TempDir;

        // Create a temp directory with a small file
        let dir = TempDir::new().unwrap();
        let small_file_path = dir.path().join("small.txt");

        // Create a file smaller than 500KB threshold
        let small_content = vec![0u8; 100 * 1024]; // 100KB
        std::fs::write(&small_file_path, small_content).unwrap();

        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--cached", "--name-only"],
            CommandOutput {
                exit_code: 0,
                stdout: format!("{}\n", small_file_path.display()),
                stderr: String::new(),
            },
        );
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );

        let violations = check_large_files(&runner);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_check_large_files_directory_ignored() {
        use tempfile::TempDir;

        // Create a temp directory
        let dir = TempDir::new().unwrap();
        let subdir = dir.path().join("subdir");
        std::fs::create_dir(&subdir).unwrap();

        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--cached", "--name-only"],
            CommandOutput {
                exit_code: 0,
                stdout: format!("{}\n", subdir.display()),
                stderr: String::new(),
            },
        );
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );

        let violations = check_large_files(&runner);
        // Directories are not flagged
        assert!(violations.is_empty());
    }
}
