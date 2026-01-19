//! Diff analysis module for detecting code quality issues.

mod empty_except;
mod large_files;
mod secrets;
mod suppression;
mod todo_check;

pub use empty_except::check_empty_except;
pub use large_files::{check_large_files, LargeFile, LARGE_FILE_THRESHOLD_BYTES};
pub use secrets::{check_hardcoded_secrets, SecretViolation};
pub use suppression::{check_error_suppression, SuppressionViolation};
pub use todo_check::{check_todo_without_issue, TodoWarning};

use crate::git::AddedLine;

/// A violation found during diff analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The file path.
    pub file: String,
    /// The line number.
    pub line_number: u32,
    /// A description of the violation.
    pub description: String,
}

impl Violation {
    /// Create a new violation.
    #[must_use]
    pub fn new(file: impl Into<String>, line_number: u32, description: impl Into<String>) -> Self {
        Self { file: file.into(), line_number, description: description.into() }
    }

    /// Format the violation for display.
    #[must_use]
    pub fn format(&self) -> String {
        format!("  {}:{}: {}", self.file, self.line_number, self.description)
    }
}

/// Results of analyzing a diff for code quality issues.
#[derive(Debug, Clone, Default)]
pub struct AnalysisResults {
    /// Error suppression violations (blocking).
    pub suppression_violations: Vec<Violation>,
    /// Empty except block violations (blocking).
    pub empty_except_violations: Vec<Violation>,
    /// Hardcoded secret violations (blocking).
    pub secret_violations: Vec<Violation>,
    /// TODO without issue reference warnings (non-blocking).
    pub todo_warnings: Vec<Violation>,
    /// Large file warnings (non-blocking).
    pub large_file_warnings: Vec<Violation>,
}

impl AnalysisResults {
    /// Check if there are any blocking violations.
    #[must_use]
    pub fn has_blocking_violations(&self) -> bool {
        !self.suppression_violations.is_empty()
            || !self.empty_except_violations.is_empty()
            || !self.secret_violations.is_empty()
    }

    /// Check if there are any warnings.
    #[must_use]
    pub fn has_warnings(&self) -> bool {
        !self.todo_warnings.is_empty() || !self.large_file_warnings.is_empty()
    }

    /// Check if there are any issues at all.
    #[must_use]
    pub fn has_issues(&self) -> bool {
        self.has_blocking_violations() || self.has_warnings()
    }
}

/// Analyze added lines for all code quality issues.
pub fn analyze_diff(added_lines: &[AddedLine]) -> AnalysisResults {
    AnalysisResults {
        suppression_violations: check_error_suppression(added_lines),
        empty_except_violations: check_empty_except(added_lines),
        secret_violations: check_hardcoded_secrets(added_lines),
        todo_warnings: check_todo_without_issue(added_lines),
        large_file_warnings: Vec::new(), // Large files checked separately
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_violation_format() {
        let v = Violation::new("src/lib.rs", 42, "test violation");
        assert_eq!(v.format(), "  src/lib.rs:42: test violation");
    }

    #[test]
    fn test_analysis_results_has_blocking() {
        let mut results = AnalysisResults::default();
        assert!(!results.has_blocking_violations());

        results.suppression_violations.push(Violation::new("f", 1, "test"));
        assert!(results.has_blocking_violations());
    }

    #[test]
    fn test_analysis_results_has_warnings() {
        let mut results = AnalysisResults::default();
        assert!(!results.has_warnings());

        results.todo_warnings.push(Violation::new("f", 1, "test"));
        assert!(results.has_warnings());
    }
}
