//! Empty except block detection.

// Python file extensions are always lowercase in practice
#![allow(clippy::case_sensitive_file_extension_comparisons)]

use crate::analysis::Violation;
use crate::git::AddedLine;

/// Check for empty or pass-only except blocks in added lines.
///
/// This detects patterns like:
/// - `except: pass`
/// - `except Exception: ...`
/// - bare `except:` clauses
pub fn check_empty_except(added_lines: &[AddedLine]) -> Vec<Violation> {
    let mut violations = Vec::new();

    for line in added_lines {
        let stripped = line.content.trim();

        // Skip non-Python files based on extension
        if !line.file.ends_with(".py") && !line.file.ends_with(".pyx") {
            continue;
        }

        // Check for patterns indicating empty/swallowed exceptions
        if stripped.contains("except") {
            // Pattern: "except: pass" or "except Exception: pass"
            if stripped.ends_with(": pass") || stripped.ends_with(": ...") {
                violations.push(Violation::new(
                    &line.file,
                    line.line_number,
                    "empty except block (swallows exception)",
                ));
            }
            // Pattern: bare "except:" (too broad)
            else if stripped == "except:" {
                violations.push(Violation::new(
                    &line.file,
                    line.line_number,
                    "bare except clause (catches all exceptions including SystemExit)",
                ));
            }
            // Pattern: "except Exception:" without specific handling
            else if stripped == "except Exception:" || stripped == "except BaseException:" {
                violations.push(Violation::new(
                    &line.file,
                    line.line_number,
                    "broad except clause (consider catching specific exceptions)",
                ));
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
    fn test_detect_except_pass() {
        let lines = vec![make_line("test.py", 10, "    except: pass")];
        let violations = check_empty_except(&lines);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].description.contains("empty except"));
    }

    #[test]
    fn test_detect_except_ellipsis() {
        let lines = vec![make_line("test.py", 10, "    except ValueError: ...")];
        let violations = check_empty_except(&lines);
        assert_eq!(violations.len(), 1);
    }

    #[test]
    fn test_detect_bare_except() {
        let lines = vec![make_line("test.py", 5, "except:")];
        let violations = check_empty_except(&lines);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].description.contains("bare except"));
    }

    #[test]
    fn test_detect_broad_exception() {
        let lines = vec![make_line("test.py", 5, "except Exception:")];
        let violations = check_empty_except(&lines);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].description.contains("broad"));
    }

    #[test]
    fn test_skip_non_python_files() {
        let lines = vec![
            make_line("test.js", 10, "except: pass"),
            make_line("test.rs", 10, "except: pass"),
        ];
        let violations = check_empty_except(&lines);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_no_false_positives() {
        let lines = vec![
            make_line("test.py", 1, "except ValueError as e:"),
            make_line("test.py", 2, "    logger.error(e)"),
            make_line("test.py", 3, "    raise"),
            make_line("test.py", 4, "# except: pass in comment"),
        ];
        let violations = check_empty_except(&lines);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_pyx_files_checked() {
        let lines = vec![make_line("fast.pyx", 10, "    except: pass")];
        let violations = check_empty_except(&lines);
        assert_eq!(violations.len(), 1);
    }
}
