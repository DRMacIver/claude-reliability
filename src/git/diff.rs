//! Git diff parsing.

use regex::Regex;
use std::sync::OnceLock;

/// A line that was added in a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddedLine {
    /// The file path.
    pub file: String,
    /// The line number in the new file.
    pub line_number: u32,
    /// The content of the line (without the leading '+').
    pub content: String,
}

/// A hunk from a unified diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffHunk {
    /// The file being modified.
    pub file: String,
    /// The starting line number in the new file.
    pub new_start: u32,
    /// Lines added in this hunk.
    pub added_lines: Vec<AddedLine>,
}

/// Regex for parsing diff file headers.
fn diff_header_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"^diff --git a/(.+) b/(.+)$").unwrap())
}

/// Regex for parsing hunk headers.
fn hunk_header_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"^@@.*\+(\d+)").unwrap())
}

/// Parse a unified diff and extract added lines.
///
/// # Arguments
///
/// * `diff` - The unified diff output from git.
///
/// # Returns
///
/// A vector of all added lines with their file paths and line numbers.
pub fn parse_diff(diff: &str) -> Vec<AddedLine> {
    let mut added_lines = Vec::new();
    let mut current_file = String::new();
    let mut current_line_num: u32 = 0;

    let header_re = diff_header_regex();
    let hunk_re = hunk_header_regex();

    for line in diff.lines() {
        if let Some(caps) = header_re.captures(line) {
            // New file - extract from "b/path"
            current_file = caps.get(2).map(|m| m.as_str().to_string()).unwrap_or_default();
        } else if let Some(caps) = hunk_re.captures(line) {
            // Hunk header - extract new file start line
            if let Some(start) = caps.get(1) {
                current_line_num = start.as_str().parse().unwrap_or(1);
            }
        } else if let Some(content) = line.strip_prefix('+') {
            // Added line (but not the +++ header)
            if !content.starts_with("++") {
                added_lines.push(AddedLine {
                    file: current_file.clone(),
                    line_number: current_line_num,
                    content: content.to_string(),
                });
                current_line_num += 1;
            }
        } else if !line.starts_with('-') && !line.starts_with('\\') {
            // Context line or other - increment line number
            if !line.starts_with("diff ") && !line.starts_with("index ") && !line.starts_with("---")
            {
                current_line_num += 1;
            }
        }
    }

    added_lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_diff() {
        let diff = r"diff --git a/src/lib.rs b/src/lib.rs
index 1234567..abcdefg 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,4 @@
 //! Library.
+// Added comment

 fn main() {}
";
        let added = parse_diff(diff);
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].file, "src/lib.rs");
        assert_eq!(added[0].line_number, 2);
        assert_eq!(added[0].content, "// Added comment");
    }

    #[test]
    fn test_parse_diff_multiple_files() {
        let diff = r"diff --git a/src/a.rs b/src/a.rs
--- a/src/a.rs
+++ b/src/a.rs
@@ -1 +1,2 @@
 line1
+line2
diff --git a/src/b.rs b/src/b.rs
--- a/src/b.rs
+++ b/src/b.rs
@@ -5 +5,2 @@
 line5
+line6
";
        let added = parse_diff(diff);
        assert_eq!(added.len(), 2);
        assert_eq!(added[0].file, "src/a.rs");
        assert_eq!(added[0].line_number, 2);
        assert_eq!(added[0].content, "line2");
        assert_eq!(added[1].file, "src/b.rs");
        assert_eq!(added[1].line_number, 6);
        assert_eq!(added[1].content, "line6");
    }

    #[test]
    fn test_parse_diff_multiple_hunks() {
        let diff = r"diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1 +1,2 @@
 first
+second
@@ -10 +11,2 @@
 tenth
+eleventh
";
        let added = parse_diff(diff);
        assert_eq!(added.len(), 2);
        assert_eq!(added[0].line_number, 2);
        assert_eq!(added[1].line_number, 12);
    }

    #[test]
    fn test_parse_empty_diff() {
        let added = parse_diff("");
        assert!(added.is_empty());
    }

    #[test]
    fn test_parse_diff_with_deletions_only() {
        let diff = r"diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,2 @@
 first
-second
 third
";
        let added = parse_diff(diff);
        assert!(added.is_empty());
    }
}
