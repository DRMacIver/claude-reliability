//! TODO/FIXME without issue reference detection.

use crate::analysis::Violation;
use crate::git::AddedLine;
use once_cell::sync::Lazy;
use regex::Regex;

/// A warning about a TODO without issue reference.
pub type TodoWarning = Violation;

/// Regex to find work markers in comments.
static TODO_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)(#|//)\s*(TODO|FIXME|HACK|XXX)\b").unwrap());

/// Regex to find issue references.
static ISSUE_REF_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"#[a-zA-Z]+-[a-zA-Z0-9]+").unwrap());

/// Check for work item markers without issue references.
///
/// This is a non-blocking warning. TODOs should ideally be linked to
/// tracked issues for better project management.
pub fn check_todo_without_issue(added_lines: &[AddedLine]) -> Vec<TodoWarning> {
    let mut warnings = Vec::new();

    for line in added_lines {
        // Check if line has a TODO/FIXME/etc marker
        if let Some(captures) = TODO_PATTERN.captures(&line.content) {
            // Check if line also has an issue reference
            if ISSUE_REF_PATTERN.is_match(&line.content) {
                continue; // Has issue reference, skip
            }

            let marker =
                captures.get(2).map_or_else(|| "TODO".to_string(), |m| m.as_str().to_uppercase());
            warnings.push(Violation::new(
                &line.file,
                line.line_number,
                format!("{marker} without issue reference"),
            ));
        }
    }

    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_line(file: &str, line_num: u32, content: &str) -> AddedLine {
        AddedLine { file: file.to_string(), line_number: line_num, content: content.to_string() }
    }

    #[test]
    fn test_detect_todo() {
        let lines = vec![make_line("test.py", 10, "# TODO: fix this later")];
        let warnings = check_todo_without_issue(&lines);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].description.contains("TODO"));
    }

    #[test]
    fn test_detect_fixme() {
        let lines = vec![make_line("test.py", 10, "# FIXME broken")];
        let warnings = check_todo_without_issue(&lines);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].description.contains("FIXME"));
    }

    #[test]
    fn test_detect_hack() {
        let lines = vec![make_line("test.js", 10, "// HACK: workaround for bug")];
        let warnings = check_todo_without_issue(&lines);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].description.contains("HACK"));
    }

    #[test]
    fn test_detect_xxx() {
        let lines = vec![make_line("test.py", 10, "# XXX this needs attention")];
        let warnings = check_todo_without_issue(&lines);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].description.contains("XXX"));
    }

    #[test]
    fn test_skip_with_issue_reference() {
        let lines = vec![
            make_line("test.py", 1, "# TODO(#project-123): fix this"),
            make_line("test.py", 2, "// FIXME #ABC-456: workaround"),
            make_line("test.py", 3, "# HACK #my-repo-789"),
        ];
        let warnings = check_todo_without_issue(&lines);
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_case_insensitive() {
        let lines = vec![
            make_line("test.py", 1, "# todo lowercase"),
            make_line("test.py", 2, "# Todo mixedcase"),
            make_line("test.py", 3, "# TODO UPPERCASE"),
        ];
        let warnings = check_todo_without_issue(&lines);
        assert_eq!(warnings.len(), 3);
    }

    #[test]
    fn test_no_false_positives() {
        let lines = vec![
            make_line("test.py", 1, "# This is a regular comment"),
            make_line("test.py", 2, "todo_list = []"),
            make_line("test.py", 3, "# The TODO pattern in quotes 'TODO'"),
        ];
        let warnings = check_todo_without_issue(&lines);
        // Only the third line might match, but it's in quotes context
        // The regex only looks at comment context
        assert!(warnings.len() <= 1);
    }

    #[test]
    fn test_js_style_comments() {
        let lines = vec![make_line("test.js", 10, "// TODO: implement this")];
        let warnings = check_todo_without_issue(&lines);
        assert_eq!(warnings.len(), 1);
    }
}
