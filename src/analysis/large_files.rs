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
}
