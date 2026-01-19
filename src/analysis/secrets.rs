//! Hardcoded secrets detection.

use crate::analysis::Violation;
use crate::git::AddedLine;
use once_cell::sync::Lazy;
use regex::Regex;

/// A detected hardcoded secret.
pub type SecretViolation = Violation;

/// Patterns for detecting hardcoded secrets.
///
/// These patterns are designed to detect real secrets while minimizing
/// false positives. They focus on well-known token formats.
static SECRET_PATTERNS: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    vec![
        // GitHub tokens (various types)
        (
            Regex::new(r"gh[spo]_[A-Za-z0-9]{36,}").unwrap(),
            "GitHub token",
        ),
        (
            Regex::new(r"ghu_[A-Za-z0-9]{36,}").unwrap(),
            "GitHub user token",
        ),
        (
            Regex::new(r"ghr_[A-Za-z0-9]{36,}").unwrap(),
            "GitHub refresh token",
        ),
        (
            Regex::new(r"github_pat_[A-Za-z0-9_]{22,}").unwrap(),
            "GitHub fine-grained PAT",
        ),
        // AWS credentials
        (
            Regex::new(r"AKIA[0-9A-Z]{16}").unwrap(),
            "AWS access key",
        ),
        // Anthropic API keys
        (
            Regex::new(r"sk-ant-[A-Za-z0-9_-]{40,}").unwrap(),
            "Anthropic API key",
        ),
        // OpenAI API keys
        (
            Regex::new(r"sk-[A-Za-z0-9]{48,}").unwrap(),
            "OpenAI API key",
        ),
        // Generic high-entropy strings that look like API keys
        // (32+ hex chars or base64 chars in a suspicious context)
        (
            Regex::new(r#"(?i)(api[_-]?key|secret|token|password)\s*[:=]\s*["']?[A-Za-z0-9+/=_-]{32,}["']?"#).unwrap(),
            "Possible API key or secret",
        ),
    ]
});

/// Check for hardcoded secrets in added lines.
///
/// This is a BLOCKING check - secrets should never be committed.
pub fn check_hardcoded_secrets(added_lines: &[AddedLine]) -> Vec<SecretViolation> {
    let mut violations = Vec::new();

    for line in added_lines {
        // Skip files in .credentials directory (expected to have secrets)
        if line.file.contains(".credentials") {
            continue;
        }

        // Skip test files that might have example/mock secrets
        let is_test_file = line.file.contains("test")
            || line.file.contains("spec")
            || line.file.contains("mock")
            || line.file.contains("fixture");

        for (regex, description) in SECRET_PATTERNS.iter() {
            if regex.is_match(&line.content) {
                // For test files, only flag if it's a known token format (not generic)
                if is_test_file && description.starts_with("Possible") {
                    continue;
                }

                violations.push(Violation::new(&line.file, line.line_number, *description));
                break; // One violation per line
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
    fn test_detect_github_token() {
        let lines =
            vec![make_line("config.py", 10, "token = 'ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'")];
        let violations = check_hardcoded_secrets(&lines);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].description.contains("GitHub"));
    }

    #[test]
    fn test_detect_github_pat() {
        let lines = vec![make_line("config.py", 10, "pat = 'github_pat_xxxxxxxxxxxxxxxxxxxxxx'")];
        let violations = check_hardcoded_secrets(&lines);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].description.contains("GitHub"));
    }

    #[test]
    fn test_detect_aws_key() {
        let lines = vec![make_line("config.py", 5, "AWS_ACCESS_KEY = 'AKIAIOSFODNN7EXAMPLE'")];
        let violations = check_hardcoded_secrets(&lines);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].description.contains("AWS"));
    }

    #[test]
    fn test_detect_anthropic_key() {
        let lines = vec![make_line(
            "config.py",
            5,
            "key = 'sk-ant-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'",
        )];
        let violations = check_hardcoded_secrets(&lines);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].description.contains("Anthropic"));
    }

    #[test]
    fn test_skip_credentials_directory() {
        let lines = vec![make_line(
            ".credentials/tokens.json",
            1,
            "ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
        )];
        let violations = check_hardcoded_secrets(&lines);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_no_false_positives() {
        let lines = vec![
            make_line("readme.md", 1, "Use your own API key"),
            make_line("config.py", 2, "key = os.environ['API_KEY']"),
            make_line("main.py", 3, "# This is not a secret"),
        ];
        let violations = check_hardcoded_secrets(&lines);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_detect_generic_api_key() {
        let lines =
            vec![make_line("config.py", 10, "api_key = 'abcdefghijklmnopqrstuvwxyz123456'")];
        let violations = check_hardcoded_secrets(&lines);
        assert_eq!(violations.len(), 1);
    }

    #[test]
    fn test_skip_generic_in_test_files() {
        let lines =
            vec![make_line("test_config.py", 10, "api_key = 'abcdefghijklmnopqrstuvwxyz123456'")];
        let violations = check_hardcoded_secrets(&lines);
        // Generic patterns are skipped in test files
        assert!(violations.is_empty());
    }

    #[test]
    fn test_real_tokens_detected_in_test_files() {
        // But real token formats are still detected
        let lines = vec![make_line(
            "test_config.py",
            10,
            "token = 'ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'",
        )];
        let violations = check_hardcoded_secrets(&lines);
        assert_eq!(violations.len(), 1);
    }
}
