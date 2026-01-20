//! Code review hook for pre-commit reviews.
//!
//! This hook runs before `git commit` commands and invokes a Claude sub-agent
//! to review the staged changes. It can approve or reject the commit with feedback.

use crate::error::Result;
use crate::git;
use crate::hooks::{HookInput, PreToolUseOutput};
use crate::traits::{CommandRunner, SubAgent};
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::Path;

/// Path to the CLAUDE.md file containing the review guide section.
const CLAUDE_MD_PATH: &str = "CLAUDE.md";

/// Header that marks the start of the code review section.
const CODE_REVIEW_HEADER: &str = "## Code Review";

/// Source code file extensions.
static SOURCE_EXTENSIONS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        // Python
        ".py", ".pyx", ".pyi", // JavaScript/TypeScript
        ".js", ".jsx", ".ts", ".tsx", ".mjs", ".cjs", // Rust
        ".rs",  // Go
        ".go",  // Java/Kotlin/Scala
        ".java", ".kt", ".kts", ".scala", // C/C++
        ".c", ".h", ".cpp", ".hpp", ".cc", ".hh", ".cxx", ".hxx", // C#
        ".cs",  // Ruby
        ".rb",  // PHP
        ".php", // Swift/Objective-C
        ".swift", ".m", ".mm", // Web frameworks
        ".vue", ".svelte", // Shell scripts
        ".sh", ".bash", ".zsh", // Other
        ".pl", ".pm", ".lua", ".r", ".R", ".jl", ".ex", ".exs", ".erl", ".hrl", ".hs", ".lhs",
        ".ml", ".mli", ".clj", ".cljs", ".cljc", ".f90", ".f95", ".f03", ".sql", ".proto",
        ".graphql", ".gql",
    ]
    .into_iter()
    .collect()
});

/// Source code directories.
static SOURCE_DIRECTORIES: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        "src",
        "lib",
        "app",
        "pkg",
        "cmd",
        "internal",
        "core",
        "test",
        "tests",
        "spec",
        "specs",
        "__tests__",
        "components",
        "pages",
        "routes",
        "handlers",
        "services",
        "models",
        "views",
        "controllers",
        "utils",
        "helpers",
    ]
    .into_iter()
    .collect()
});

/// Directories to always exclude.
static EXCLUDED_DIRECTORIES: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        ".beads",
        ".claude",
        ".git",
        ".github",
        ".vscode",
        "node_modules",
        "vendor",
        "__pycache__",
        ".mypy_cache",
        "dist",
        "build",
        "target",
        ".next",
        ".nuxt",
        "coverage",
        ".pytest_cache",
        ".tox",
        "venv",
        ".venv",
        "eggs",
    ]
    .into_iter()
    .collect()
});

/// Regex for detecting git commit commands (but not --amend).
static GIT_COMMIT_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"\bgit\s+commit\b").unwrap());

/// Configuration for the code review hook.
#[derive(Debug, Clone, Default)]
pub struct CodeReviewConfig {
    /// Whether to skip the review (e.g., `SKIP_CODE_REVIEW` env var).
    pub skip_review: bool,
}

/// Check if a file is source code based on heuristics.
pub fn is_source_code_file(filepath: &str) -> bool {
    let path = Path::new(filepath);

    // Check if in excluded directory
    for component in path.components() {
        let part = component.as_os_str().to_string_lossy();
        if EXCLUDED_DIRECTORIES.contains(part.as_ref()) || part.ends_with(".egg-info") {
            return false;
        }
    }

    // Check extension
    if let Some(ext) = path.extension() {
        let ext_with_dot = format!(".{}", ext.to_string_lossy().to_lowercase());
        if SOURCE_EXTENSIONS.contains(ext_with_dot.as_str()) {
            return true;
        }
    }

    // Check if in source directory
    for component in path.components() {
        let part = component.as_os_str().to_string_lossy().to_lowercase();
        if SOURCE_DIRECTORIES.contains(part.as_str()) {
            return true;
        }
    }

    false
}

/// Load the review guide from the "Code Review" section of CLAUDE.md.
///
/// Extracts the content from the `## Code Review` header until the next `## ` header
/// or end of file.
pub fn load_review_guide() -> Option<String> {
    let content = fs::read_to_string(CLAUDE_MD_PATH).ok()?;
    extract_code_review_section(&content)
}

/// Extract the "Code Review" section from CLAUDE.md content.
///
/// Returns the content from `## Code Review` to the next `## ` header (exclusive).
fn extract_code_review_section(content: &str) -> Option<String> {
    // Find the start of the Code Review section
    let start_idx = content.find(CODE_REVIEW_HEADER)?;
    let section_start = start_idx + CODE_REVIEW_HEADER.len();

    // Find the next ## header (end of section) or use end of content
    let remaining = &content[section_start..];
    let section_end = remaining.find("\n## ").map_or(content.len(), |idx| section_start + idx);

    let section = content[section_start..section_end].trim();
    if section.is_empty() {
        None
    } else {
        Some(section.to_string())
    }
}

/// Run the code review hook.
///
/// Returns exit code: 0 = allow (with optional feedback), 2 = reject.
///
/// # Errors
///
/// Returns an error if git commands or sub-agent calls fail.
pub fn run_code_review_hook(
    input: &HookInput,
    config: &CodeReviewConfig,
    runner: &dyn CommandRunner,
    sub_agent: &dyn SubAgent,
) -> Result<i32> {
    // Skip if disabled
    if config.skip_review {
        return Ok(0);
    }

    // Only run for Bash tool calls
    if input.tool_name.as_deref() != Some("Bash") {
        return Ok(0);
    }

    // Get the command
    let command = input.tool_input.as_ref().and_then(|t| t.command.as_deref()).unwrap_or("");

    // Check if this is a git commit command
    if !GIT_COMMIT_REGEX.is_match(command) {
        return Ok(0);
    }

    // Skip review for --amend commits
    if command.contains("--amend") {
        return Ok(0);
    }

    // Get staged files
    let staged_files = git::staged_files(runner)?;
    if staged_files.is_empty() {
        return Ok(0);
    }

    // Filter for source code files
    let source_files: Vec<String> =
        staged_files.into_iter().filter(|f| is_source_code_file(f)).collect();

    if source_files.is_empty() {
        // No source code files, allow the commit
        return Ok(0);
    }

    // Get the diff for review
    let diff = git::staged_diff(runner)?;
    if diff.is_empty() {
        return Ok(0);
    }

    // Load review guide
    let review_guide = load_review_guide();

    // Run the review
    eprintln!("Running code review for {} source file(s)...", source_files.len());

    let (approved, feedback) =
        sub_agent.review_code(&diff, &source_files, review_guide.as_deref())?;

    if approved {
        // Approved - provide feedback if any
        if !feedback.is_empty() && !feedback.starts_with("Code review") {
            let output =
                PreToolUseOutput::allow(Some(format!("Code Review Feedback:\n{feedback}")));
            println!("{}", serde_json::to_string(&output)?);
        }
        Ok(0)
    } else {
        // Rejected - block the commit
        let mut stderr = std::io::stderr();
        writeln!(stderr)?;
        writeln!(stderr, "{}", "=".repeat(60))?;
        writeln!(stderr, "CODE REVIEW: REJECTED")?;
        writeln!(stderr, "{}", "=".repeat(60))?;
        writeln!(stderr)?;
        writeln!(stderr, "{feedback}")?;
        writeln!(stderr)?;
        writeln!(stderr, "Please address the review feedback before committing.")?;
        writeln!(stderr, "Set SKIP_CODE_REVIEW=1 to bypass (not recommended).")?;
        writeln!(stderr)?;
        Ok(2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_source_code_file_by_extension() {
        assert!(is_source_code_file("src/main.rs"));
        assert!(is_source_code_file("lib/utils.py"));
        assert!(is_source_code_file("app.js"));
        assert!(is_source_code_file("Component.tsx"));
    }

    #[test]
    fn test_is_source_code_file_by_directory() {
        assert!(is_source_code_file("src/foo/bar.txt"));
        assert!(is_source_code_file("tests/test_foo.txt"));
        assert!(is_source_code_file("components/Button.txt"));
    }

    #[test]
    fn test_is_source_code_file_excluded() {
        assert!(!is_source_code_file("node_modules/package/index.js"));
        assert!(!is_source_code_file(".beads/issue.yaml"));
        assert!(!is_source_code_file("vendor/lib.py"));
        assert!(!is_source_code_file("__pycache__/foo.pyc"));
    }

    #[test]
    fn test_is_source_code_file_not_source() {
        assert!(!is_source_code_file("README.md"));
        assert!(!is_source_code_file("config.yaml"));
        assert!(!is_source_code_file("data.json"));
        assert!(!is_source_code_file("image.png"));
    }

    #[test]
    fn test_git_commit_regex() {
        assert!(GIT_COMMIT_REGEX.is_match("git commit -m 'test'"));
        assert!(GIT_COMMIT_REGEX.is_match("git commit -am 'test'"));
        assert!(!GIT_COMMIT_REGEX.is_match("git status"));
        assert!(!GIT_COMMIT_REGEX.is_match("git push"));
    }

    #[test]
    fn test_is_source_code_file_egg_info() {
        assert!(!is_source_code_file("mypackage.egg-info/PKG-INFO"));
    }

    #[test]
    fn test_run_code_review_hook_skip_review() {
        use crate::testing::{MockCommandRunner, MockSubAgent};

        let runner = MockCommandRunner::new();
        let sub_agent = MockSubAgent::new();
        let input = HookInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(crate::hooks::ToolInput {
                command: Some("git commit -m 'test'".to_string()),
            }),
            ..Default::default()
        };
        let config = CodeReviewConfig { skip_review: true };

        let result = run_code_review_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert_eq!(result, 0);
    }

    #[test]
    fn test_run_code_review_hook_not_bash() {
        use crate::testing::{MockCommandRunner, MockSubAgent};

        let runner = MockCommandRunner::new();
        let sub_agent = MockSubAgent::new();
        let input = HookInput { tool_name: Some("Read".to_string()), ..Default::default() };
        let config = CodeReviewConfig::default();

        let result = run_code_review_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert_eq!(result, 0);
    }

    #[test]
    fn test_run_code_review_hook_not_commit() {
        use crate::testing::{MockCommandRunner, MockSubAgent};

        let runner = MockCommandRunner::new();
        let sub_agent = MockSubAgent::new();
        let input = HookInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(crate::hooks::ToolInput { command: Some("git status".to_string()) }),
            ..Default::default()
        };
        let config = CodeReviewConfig::default();

        let result = run_code_review_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert_eq!(result, 0);
    }

    #[test]
    fn test_run_code_review_hook_amend() {
        use crate::testing::{MockCommandRunner, MockSubAgent};

        let runner = MockCommandRunner::new();
        let sub_agent = MockSubAgent::new();
        let input = HookInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(crate::hooks::ToolInput {
                command: Some("git commit --amend -m 'test'".to_string()),
            }),
            ..Default::default()
        };
        let config = CodeReviewConfig::default();

        let result = run_code_review_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert_eq!(result, 0);
    }

    #[test]
    fn test_run_code_review_hook_no_staged_files() {
        use crate::testing::{MockCommandRunner, MockSubAgent};
        use crate::traits::CommandOutput;

        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--cached", "--name-only"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        let sub_agent = MockSubAgent::new();
        let input = HookInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(crate::hooks::ToolInput {
                command: Some("git commit -m 'test'".to_string()),
            }),
            ..Default::default()
        };
        let config = CodeReviewConfig::default();

        let result = run_code_review_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert_eq!(result, 0);
    }

    #[test]
    fn test_run_code_review_hook_no_source_files() {
        use crate::testing::{MockCommandRunner, MockSubAgent};
        use crate::traits::CommandOutput;

        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--cached", "--name-only"],
            CommandOutput {
                exit_code: 0,
                stdout: "README.md\nconfig.yaml\n".to_string(),
                stderr: String::new(),
            },
        );
        let sub_agent = MockSubAgent::new();
        let input = HookInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(crate::hooks::ToolInput {
                command: Some("git commit -m 'test'".to_string()),
            }),
            ..Default::default()
        };
        let config = CodeReviewConfig::default();

        let result = run_code_review_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert_eq!(result, 0);
    }

    #[test]
    fn test_run_code_review_hook_empty_diff() {
        use crate::testing::{MockCommandRunner, MockSubAgent};
        use crate::traits::CommandOutput;

        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--cached", "--name-only"],
            CommandOutput {
                exit_code: 0,
                stdout: "src/main.rs\n".to_string(),
                stderr: String::new(),
            },
        );
        runner.expect(
            "git",
            &["diff", "--cached", "-U0"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        let sub_agent = MockSubAgent::new();
        let input = HookInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(crate::hooks::ToolInput {
                command: Some("git commit -m 'test'".to_string()),
            }),
            ..Default::default()
        };
        let config = CodeReviewConfig::default();

        let result = run_code_review_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert_eq!(result, 0);
    }

    #[test]
    fn test_run_code_review_hook_approved() {
        use crate::testing::{MockCommandRunner, MockSubAgent};
        use crate::traits::CommandOutput;

        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--cached", "--name-only"],
            CommandOutput {
                exit_code: 0,
                stdout: "src/main.rs\n".to_string(),
                stderr: String::new(),
            },
        );
        runner.expect(
            "git",
            &["diff", "--cached", "-U0"],
            CommandOutput {
                exit_code: 0,
                stdout: "+fn main() {}\n".to_string(),
                stderr: String::new(),
            },
        );
        let mut sub_agent = MockSubAgent::new();
        sub_agent.expect_review(true, "LGTM");

        let input = HookInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(crate::hooks::ToolInput {
                command: Some("git commit -m 'test'".to_string()),
            }),
            ..Default::default()
        };
        let config = CodeReviewConfig::default();

        let result = run_code_review_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert_eq!(result, 0);
    }

    #[test]
    fn test_run_code_review_hook_rejected() {
        use crate::testing::{MockCommandRunner, MockSubAgent};
        use crate::traits::CommandOutput;

        let mut runner = MockCommandRunner::new();
        runner.expect(
            "git",
            &["diff", "--cached", "--name-only"],
            CommandOutput {
                exit_code: 0,
                stdout: "src/main.rs\n".to_string(),
                stderr: String::new(),
            },
        );
        runner.expect(
            "git",
            &["diff", "--cached", "-U0"],
            CommandOutput {
                exit_code: 0,
                stdout: "+fn main() {}\n".to_string(),
                stderr: String::new(),
            },
        );
        let mut sub_agent = MockSubAgent::new();
        sub_agent.expect_review(false, "Security issue found");

        let input = HookInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(crate::hooks::ToolInput {
                command: Some("git commit -m 'test'".to_string()),
            }),
            ..Default::default()
        };
        let config = CodeReviewConfig::default();

        let result = run_code_review_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert_eq!(result, 2);
    }

    #[test]
    fn test_run_code_review_hook_no_tool_input() {
        use crate::testing::{MockCommandRunner, MockSubAgent};

        let runner = MockCommandRunner::new();
        let sub_agent = MockSubAgent::new();
        let input = HookInput {
            tool_name: Some("Bash".to_string()),
            tool_input: None,
            ..Default::default()
        };
        let config = CodeReviewConfig::default();

        let result = run_code_review_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert_eq!(result, 0);
    }

    #[test]
    fn test_load_review_guide_from_claude_md() {
        // When CLAUDE.md exists with Code Review section, should return content
        let guide = load_review_guide();
        // This depends on whether CLAUDE.md exists in the repo with the section
        // Just make sure it doesn't panic
        let _ = guide;
    }

    #[test]
    fn test_extract_code_review_section_found() {
        let content = r"# Project

## Development

Some dev content.

## Code Review

This is the review guide content.

### What to Check

- Security issues
- Logic errors

## Other Section

This should not be included.
";
        let section = extract_code_review_section(content).unwrap();
        assert!(section.contains("This is the review guide content."));
        assert!(section.contains("What to Check"));
        assert!(section.contains("Security issues"));
        assert!(!section.contains("Other Section"));
        assert!(!section.contains("This should not be included"));
    }

    #[test]
    fn test_extract_code_review_section_at_end() {
        let content = r"# Project

## Code Review

This is the only content.
";
        let section = extract_code_review_section(content).unwrap();
        assert!(section.contains("This is the only content."));
    }

    #[test]
    fn test_extract_code_review_section_not_found() {
        let content = r"# Project

## Development

Some content.
";
        assert!(extract_code_review_section(content).is_none());
    }

    #[test]
    fn test_extract_code_review_section_empty() {
        let content = r"# Project

## Code Review

## Next Section
";
        // Empty section (only whitespace) should return None
        assert!(extract_code_review_section(content).is_none());
    }
}
