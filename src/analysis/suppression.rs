//! Error suppression detection.

#![allow(clippy::trivial_regex)] // Regex used for consistency across patterns

use crate::analysis::Violation;
use crate::git::AddedLine;
use once_cell::sync::Lazy;
use regex::Regex;

/// A detected error suppression.
pub type SuppressionViolation = Violation;

/// Patterns for detecting error suppression directives.
static SUPPRESSION_PATTERNS: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    vec![
        // Python suppression directives
        (Regex::new(r"#\s*type:\s*ignore").unwrap(), "type: ignore comment"),
        (Regex::new(r"#\s*noqa").unwrap(), "noqa comment"),
        (Regex::new(r"#\s*pylint:\s*disable").unwrap(), "pylint disable comment"),
        // TypeScript/JavaScript suppression directives
        (Regex::new(r"//\s*@ts-ignore").unwrap(), "TypeScript @ts-ignore comment"),
        (Regex::new(r"//\s*@ts-expect-error").unwrap(), "TypeScript @ts-expect-error comment"),
        // ESLint suppression directives
        (Regex::new(r"(/\*|//)\s*eslint-disable").unwrap(), "ESLint disable comment"),
        // Rust suppression (allow attributes on same line as code)
        (Regex::new(r"#\[allow\(").unwrap(), "Rust #[allow(...)] attribute"),
    ]
});

/// Check for error suppression patterns in added lines.
///
/// This detects patterns like `# type: ignore`, `// @ts-ignore`, etc.
/// that suppress linter/type checker errors.
pub fn check_error_suppression(added_lines: &[AddedLine]) -> Vec<SuppressionViolation> {
    let mut violations = Vec::new();

    for line in added_lines {
        for (regex, description) in SUPPRESSION_PATTERNS.iter() {
            if regex.is_match(&line.content) {
                violations.push(Violation::new(&line.file, line.line_number, *description));
                break; // One violation per line is enough
            }
        }
    }

    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_line(file: &str, line_num: u32, content: &str) -> AddedLine {
        AddedLine { file: file.to_string(), line_number: line_num, content: content.to_string() }
    }

    #[test]
    fn test_detect_type_ignore() {
        let lines = vec![make_line("test.py", 10, "x = foo()  # type: ignore")];
        let violations = check_error_suppression(&lines);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].description.contains("type: ignore"));
    }

    #[test]
    fn test_detect_noqa() {
        let lines = vec![make_line("test.py", 20, "import foo  # noqa: F401")];
        let violations = check_error_suppression(&lines);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].description.contains("noqa"));
    }

    #[test]
    fn test_detect_ts_ignore() {
        let lines = vec![make_line("test.ts", 5, "// @ts-ignore")];
        let violations = check_error_suppression(&lines);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].description.contains("@ts-ignore"));
    }

    #[test]
    fn test_detect_ts_expect_error() {
        let lines = vec![make_line("test.ts", 5, "// @ts-expect-error")];
        let violations = check_error_suppression(&lines);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].description.contains("@ts-expect-error"));
    }

    #[test]
    fn test_detect_eslint_disable() {
        let lines = vec![
            make_line("test.js", 1, "// eslint-disable-next-line"),
            make_line("test.js", 2, "/* eslint-disable no-unused-vars */"),
        ];
        let violations = check_error_suppression(&lines);
        assert_eq!(violations.len(), 2);
    }

    #[test]
    fn test_detect_pylint_disable() {
        let lines = vec![make_line("test.py", 15, "x = 1  # pylint: disable=invalid-name")];
        let violations = check_error_suppression(&lines);
        assert_eq!(violations.len(), 1);
    }

    #[test]
    fn test_no_false_positives() {
        let lines = vec![
            make_line("test.py", 1, "# This is a normal comment"),
            make_line("test.py", 2, "x = 'ignore this'"),
            make_line("test.js", 3, "// Regular comment"),
            make_line("test.ts", 4, "const ts = 'typescript';"),
        ];
        let violations = check_error_suppression(&lines);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_one_violation_per_line() {
        // Even if a line matches multiple patterns, we only report one
        let lines = vec![make_line("test.py", 1, "# type: ignore  # noqa")];
        let violations = check_error_suppression(&lines);
        assert_eq!(violations.len(), 1);
    }
}
