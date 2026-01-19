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

/// Path to the review guide file.
const REVIEW_GUIDE_PATH: &str = "REVIEWGUIDE.md";

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

/// Load the review guide if it exists.
pub fn load_review_guide() -> Option<String> {
    fs::read_to_string(REVIEW_GUIDE_PATH).ok()
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
}
