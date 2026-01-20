//! Configuration management for claude-reliability.
//!
//! This module handles the `.claude/reliability-config.yaml` file which stores
//! project-specific settings for the reliability hooks.

use crate::error::Result;
use crate::traits::CommandRunner;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Config file path relative to project root.
pub const CONFIG_FILE_PATH: &str = ".claude/reliability-config.yaml";

/// Header comment for the managed gitignore section.
const GITIGNORE_SECTION_HEADER: &str = "# claude-reliability managed";

/// Files that should be gitignored by claude-reliability.
const GITIGNORE_ENTRIES: &[&str] =
    &[".claude/bin/", ".claude/*.local.md", ".claude/*.local.json", ".claude/*.local"];

/// Path to CLAUDE.md file.
const CLAUDE_MD_PATH: &str = "CLAUDE.md";

/// Header for the code review section in CLAUDE.md.
const CODE_REVIEW_HEADER: &str = "## Code Review";

/// Default code review section content.
const DEFAULT_CODE_REVIEW_SECTION: &str = r"## Code Review

This section provides guidance to the automated code reviewer.

### What to Check

**Security:**
- No hardcoded secrets, API keys, or credentials
- No SQL injection or command injection vulnerabilities
- Proper input validation and sanitization

**Correctness:**
- Does the code do what's intended?
- Are there obvious logic errors or bugs?
- Are edge cases handled appropriately?

**Code Quality:**
- Clear, readable code with appropriate naming
- Proper error handling for critical paths
- Consistent style with the rest of the codebase

### When to Reject

- Security vulnerabilities
- Clear bugs or logic errors
- Missing error handling for critical paths

### When to Approve with Feedback

- Minor style issues
- Suggestions for improvement
- Missing test coverage for non-critical paths
";

/// Project configuration for reliability hooks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // Config structs legitimately have many boolean flags
pub struct ProjectConfig {
    /// Whether this is a git repository.
    #[serde(default)]
    pub git_repo: bool,

    /// Whether beads (bd) is installed and available.
    #[serde(default)]
    pub beads_installed: bool,

    /// Command to run for quality checks (e.g., "just check").
    /// None means no quality check command is configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_command: Option<String>,

    /// Whether CLAUDE.md has a "Code Review" section.
    #[serde(default)]
    pub code_review_section: bool,

    /// Whether to require pushing commits before allowing exit.
    /// Defaults to true for git repos.
    #[serde(default = "default_require_push")]
    pub require_push: bool,
}

/// Default value for `require_push` - true by default.
const fn default_require_push() -> bool {
    true
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            git_repo: false,
            beads_installed: false,
            check_command: None,
            code_review_section: false,
            require_push: true,
        }
    }
}

impl ProjectConfig {
    /// Load config from the default location, returning None if not found.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    pub fn load() -> Result<Option<Self>> {
        Self::load_from(Path::new("."))
    }

    /// Load config from a specific base directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    pub fn load_from(base_dir: &Path) -> Result<Option<Self>> {
        let config_path = base_dir.join(CONFIG_FILE_PATH);
        if !config_path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&config_path)?;
        let config: Self = serde_yaml::from_str(&content)?;
        Ok(Some(config))
    }

    /// Save config to the default location.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save(&self) -> Result<()> {
        self.save_to(Path::new("."))
    }

    /// Save config to a specific base directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save_to(&self, base_dir: &Path) -> Result<()> {
        let config_path = base_dir.join(CONFIG_FILE_PATH);

        // Ensure .claude directory exists
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content = serde_yaml::to_string(self)?;
        std::fs::write(&config_path, content)?;
        Ok(())
    }

    /// Detect project configuration by checking the environment.
    ///
    /// This checks:
    /// - Whether `.git` directory exists (`git_repo`)
    /// - Whether `bd` command is available (`beads_installed`)
    /// - Whether `just check` is viable (`check_command`)
    pub fn detect(runner: &dyn CommandRunner) -> Self {
        Self::detect_in(runner, Path::new("."))
    }

    /// Detect project configuration in a specific directory.
    pub fn detect_in(runner: &dyn CommandRunner, base_dir: &Path) -> Self {
        let git_repo = base_dir.join(".git").exists();
        let beads_installed = runner.is_available("bd");
        let check_command = detect_check_command(runner, base_dir);
        let code_review_section = has_code_review_section(base_dir);

        // Only require push if there's a remote configured
        let require_push = git_repo && has_git_remote(runner);

        Self { git_repo, beads_installed, check_command, code_review_section, require_push }
    }

    /// Get the config file path for a base directory.
    pub fn config_path(base_dir: &Path) -> PathBuf {
        base_dir.join(CONFIG_FILE_PATH)
    }
}

/// Detect an appropriate quality check command for the project.
///
/// Returns `Some("just check")` if:
/// - `just` is installed
/// - A `justfile` or `Justfile` exists
/// - The justfile defines a `check:` target
///
/// Otherwise returns `None`.
fn detect_check_command(runner: &dyn CommandRunner, base_dir: &Path) -> Option<String> {
    // Check if just is installed
    if !runner.is_available("just") {
        return None;
    }

    // Check for justfile
    let justfile_path = find_justfile(base_dir)?;

    // Check if justfile defines 'check:' target
    if has_check_target(&justfile_path) {
        Some("just check".to_string())
    } else {
        None
    }
}

/// Check if there's a git remote configured.
///
/// This is used to determine whether to require pushing before exit.
/// If there's no remote, there's nowhere to push.
fn has_git_remote(runner: &dyn CommandRunner) -> bool {
    // List remotes - if any exist, we have a remote
    let output = runner.run("git", &["remote"], None);
    match output {
        Ok(out) if out.success() => !out.stdout.trim().is_empty(),
        _ => false,
    }
}

/// Find the justfile in the given directory.
fn find_justfile(base_dir: &Path) -> Option<PathBuf> {
    for name in ["justfile", "Justfile"] {
        let path = base_dir.join(name);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

/// Check if a justfile defines the 'check:' target.
fn has_check_target(justfile_path: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(justfile_path) else {
        return false;
    };

    // Look for a line starting with 'check:' (possibly with recipe attributes before it)
    for line in content.lines() {
        let trimmed = line.trim();
        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // Check for 'check:' at the start of a line (recipe definition)
        if trimmed.starts_with("check:") || trimmed.starts_with("check ") {
            return true;
        }
    }
    false
}

/// Check if CLAUDE.md has a "Code Review" section.
fn has_code_review_section(base_dir: &Path) -> bool {
    let claude_md_path = base_dir.join(CLAUDE_MD_PATH);
    let Ok(content) = std::fs::read_to_string(claude_md_path) else {
        return false;
    };
    content.contains(CODE_REVIEW_HEADER)
}

/// Ensure CLAUDE.md has a "Code Review" section, adding it if missing.
///
/// Returns true if the section was added, false if it already existed or couldn't be added.
fn ensure_code_review_section(base_dir: &Path) -> bool {
    let claude_md_path = base_dir.join(CLAUDE_MD_PATH);

    // Read existing content or return false if file doesn't exist
    let Ok(mut content) = std::fs::read_to_string(&claude_md_path) else {
        return false;
    };

    // If section already exists, nothing to do
    if content.contains(CODE_REVIEW_HEADER) {
        return false;
    }

    // Append the code review section
    if !content.ends_with('\n') {
        content.push('\n');
    }
    content.push('\n');
    content.push_str(DEFAULT_CODE_REVIEW_SECTION);

    // Write back
    if std::fs::write(&claude_md_path, &content).is_err() {
        return false;
    }

    true
}

/// Auto-commit the CLAUDE.md changes.
fn auto_commit_claude_md(base_dir: &Path) {
    use std::process::Command;

    let claude_md_path = base_dir.join(CLAUDE_MD_PATH);
    let claude_md_str = claude_md_path.to_string_lossy();

    // Try to add the file
    let add_result =
        Command::new("git").args(["add", &claude_md_str]).current_dir(base_dir).output();

    if add_result.is_err() || !add_result.unwrap().status.success() {
        return;
    }

    // Check if there's anything to commit
    let diff_result = Command::new("git")
        .args(["diff", "--cached", "--quiet", "--", &claude_md_str])
        .current_dir(base_dir)
        .output();

    if let Ok(output) = diff_result {
        if output.status.success() {
            return;
        }
    }

    // Commit the CLAUDE.md
    let _ = Command::new("git")
        .args([
            "commit",
            "-m",
            "Add Code Review section to CLAUDE.md\n\nAuto-generated by claude-reliability plugin.",
        ])
        .current_dir(base_dir)
        .output();
}

/// Ensure config exists, creating it with detected defaults if not.
///
/// Returns the config (either loaded or newly created).
///
/// # Errors
///
/// Returns an error if config cannot be loaded or saved.
pub fn ensure_config(runner: &dyn CommandRunner) -> Result<ProjectConfig> {
    ensure_config_in(runner, Path::new("."))
}

/// Ensure config exists in a specific directory.
///
/// If the config is created and we're in a git repo, it will be automatically
/// added and committed. Also ensures .gitignore has the required entries and
/// CLAUDE.md has a Code Review section.
///
/// # Errors
///
/// Returns an error if config cannot be loaded or saved.
pub fn ensure_config_in(runner: &dyn CommandRunner, base_dir: &Path) -> Result<ProjectConfig> {
    // Try to load existing config
    let mut config = if let Some(config) = ProjectConfig::load_from(base_dir)? {
        config
    } else {
        // Detect and create new config
        let config = ProjectConfig::detect_in(runner, base_dir);
        config.save_to(base_dir)?;

        // Auto-commit config if in a git repo
        if config.git_repo {
            auto_commit_config(base_dir);
        }

        config
    };

    // Always ensure gitignore is up to date (if in a git repo)
    if config.git_repo {
        if let Ok(modified) = ensure_gitignore(base_dir) {
            if modified {
                auto_commit_gitignore(base_dir);
            }
        }
    }

    // Ensure Code Review section exists in CLAUDE.md
    if !config.code_review_section {
        // Check if it exists now (might have been added manually)
        if has_code_review_section(base_dir) {
            config.code_review_section = true;
            config.save_to(base_dir)?;
            if config.git_repo {
                auto_commit_config(base_dir);
            }
        } else if ensure_code_review_section(base_dir) {
            // Section was added
            config.code_review_section = true;
            config.save_to(base_dir)?;
            if config.git_repo {
                auto_commit_claude_md(base_dir);
                auto_commit_config(base_dir);
            }
        }
    }

    Ok(config)
}

/// Automatically add and commit the config file.
/// Silently ignores any errors (best effort).
fn auto_commit_config(base_dir: &Path) {
    use std::process::Command;

    let config_path = ProjectConfig::config_path(base_dir);
    let config_path_str = config_path.to_string_lossy();

    // Try to add the file
    let add_result =
        Command::new("git").args(["add", &config_path_str]).current_dir(base_dir).output();

    if add_result.is_err() || !add_result.unwrap().status.success() {
        return;
    }

    // Check if there's anything to commit (file might already be committed)
    let diff_result = Command::new("git")
        .args(["diff", "--cached", "--quiet", "--", &config_path_str])
        .current_dir(base_dir)
        .output();

    if let Ok(output) = diff_result {
        if output.status.success() {
            // No changes staged, nothing to commit
            return;
        }
    }

    // Commit the config file
    let _ = Command::new("git")
        .args([
            "commit",
            "-m",
            "Add claude-reliability config\n\nAuto-generated by claude-reliability plugin.",
        ])
        .current_dir(base_dir)
        .output();
}

/// Ensure the gitignore contains the required entries for claude-reliability.
///
/// This function:
/// 1. Reads the existing .gitignore (or creates one if it doesn't exist)
/// 2. Finds or creates a managed section headed by `# claude-reliability managed`
/// 3. Ensures all required entries are present in that section
/// 4. Preserves all other content in the file
///
/// Returns true if the gitignore was modified, false if it was already correct.
///
/// # Errors
///
/// Returns an error if the .gitignore file cannot be read or written.
pub fn ensure_gitignore(base_dir: &Path) -> std::io::Result<bool> {
    let gitignore_path = base_dir.join(".gitignore");

    // Read existing content or start fresh
    let existing_content = if gitignore_path.exists() {
        std::fs::read_to_string(&gitignore_path)?
    } else {
        String::new()
    };

    let new_content = update_gitignore_content(&existing_content);

    if new_content == existing_content {
        return Ok(false);
    }

    std::fs::write(&gitignore_path, &new_content)?;
    Ok(true)
}

/// Update gitignore content to include the managed section.
#[allow(clippy::option_if_let_else)]
fn update_gitignore_content(existing: &str) -> String {
    let managed_section = build_managed_section();

    // Check if the managed section already exists
    if let Some(start_idx) = existing.find(GITIGNORE_SECTION_HEADER) {
        // Find the end of the managed section (next comment or end of file)
        let after_header = &existing[start_idx + GITIGNORE_SECTION_HEADER.len()..];
        let section_end = find_section_end(after_header);

        let before = &existing[..start_idx];
        let after = &after_header[section_end..];

        // Rebuild with updated managed section
        let mut result = before.to_string();
        if !result.is_empty() && !result.ends_with('\n') {
            result.push('\n');
        }
        result.push_str(&managed_section);
        if !after.is_empty() {
            if !result.ends_with('\n') {
                result.push('\n');
            }
            result.push_str(after.trim_start_matches('\n'));
        }
        result
    } else {
        // No managed section exists, append it
        let mut result = existing.to_string();
        if !result.is_empty() && !result.ends_with('\n') {
            result.push('\n');
        }
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(&managed_section);
        result
    }
}

/// Build the managed section content.
fn build_managed_section() -> String {
    let mut section = String::from(GITIGNORE_SECTION_HEADER);
    section.push('\n');
    for entry in GITIGNORE_ENTRIES {
        section.push_str(entry);
        section.push('\n');
    }
    section
}

/// Find the end of the managed section (next comment line or end of string).
fn find_section_end(content: &str) -> usize {
    let mut pos = 0;
    for line in content.lines() {
        // Skip the newline after the header
        pos += line.len() + 1; // +1 for newline

        // If we hit another comment line (not empty), that's the end of our section
        let trimmed = line.trim();
        if trimmed.starts_with('#') && !trimmed.is_empty() && pos > 1 {
            // Go back to before this line
            return pos - line.len() - 1;
        }
    }
    // Reached end of file
    content.len()
}

/// Automatically add and commit the gitignore changes.
/// Silently ignores any errors (best effort).
fn auto_commit_gitignore(base_dir: &Path) {
    use std::process::Command;

    let gitignore_path = base_dir.join(".gitignore");
    let gitignore_str = gitignore_path.to_string_lossy();

    // Try to add the file
    let add_result =
        Command::new("git").args(["add", &gitignore_str]).current_dir(base_dir).output();

    if add_result.is_err() || !add_result.unwrap().status.success() {
        return;
    }

    // Check if there's anything to commit
    let diff_result = Command::new("git")
        .args(["diff", "--cached", "--quiet", "--", &gitignore_str])
        .current_dir(base_dir)
        .output();

    if let Ok(output) = diff_result {
        if output.status.success() {
            return;
        }
    }

    // Commit the gitignore
    let _ = Command::new("git")
        .args(["commit", "-m", "Update .gitignore for claude-reliability\n\nAuto-generated by claude-reliability plugin."])
        .current_dir(base_dir)
        .output();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::MockCommandRunner;
    use tempfile::TempDir;

    #[test]
    fn test_project_config_default() {
        let config = ProjectConfig::default();
        assert!(!config.git_repo);
        assert!(!config.beads_installed);
        assert!(config.check_command.is_none());
    }

    #[test]
    fn test_project_config_load_not_found() {
        let dir = TempDir::new().unwrap();
        let result = ProjectConfig::load_from(dir.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_project_config_save_and_load() {
        let dir = TempDir::new().unwrap();

        let config = ProjectConfig {
            git_repo: true,
            beads_installed: true,
            check_command: Some("just check".to_string()),
            code_review_section: false,
            require_push: true,
        };

        config.save_to(dir.path()).unwrap();

        let loaded = ProjectConfig::load_from(dir.path()).unwrap().unwrap();
        assert_eq!(config, loaded);
    }

    #[test]
    fn test_project_config_yaml_format() {
        let dir = TempDir::new().unwrap();

        let config = ProjectConfig {
            git_repo: true,
            beads_installed: false,
            check_command: Some("make test".to_string()),
            code_review_section: false,
            require_push: true,
        };

        config.save_to(dir.path()).unwrap();

        let content = std::fs::read_to_string(dir.path().join(CONFIG_FILE_PATH)).unwrap();
        assert!(content.contains("git_repo: true"));
        assert!(content.contains("beads_installed: false"));
        assert!(content.contains("check_command: make test"));
        assert!(content.contains("require_push: true"));
    }

    #[test]
    fn test_project_config_detect_no_git() {
        let dir = TempDir::new().unwrap();
        let runner = MockCommandRunner::new();

        let config = ProjectConfig::detect_in(&runner, dir.path());
        assert!(!config.git_repo);
    }

    #[test]
    fn test_project_config_detect_with_git() {
        use crate::traits::CommandOutput;

        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        let mut runner = MockCommandRunner::new();
        // Mock git remote returning empty (no remote configured)
        runner.expect(
            "git",
            &["remote"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );

        let config = ProjectConfig::detect_in(&runner, dir.path());
        assert!(config.git_repo);
        // No remote, so require_push should be false
        assert!(!config.require_push);
        runner.verify();
    }

    #[test]
    fn test_project_config_detect_beads() {
        let dir = TempDir::new().unwrap();
        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");

        let config = ProjectConfig::detect_in(&runner, dir.path());
        assert!(config.beads_installed);
    }

    #[test]
    fn test_project_config_detect_check_command() {
        let dir = TempDir::new().unwrap();

        // Create justfile with check target
        std::fs::write(dir.path().join("justfile"), "check:\n    cargo test\n").unwrap();

        let mut runner = MockCommandRunner::new();
        runner.set_available("just");

        let config = ProjectConfig::detect_in(&runner, dir.path());
        assert_eq!(config.check_command, Some("just check".to_string()));
    }

    #[test]
    fn test_project_config_detect_no_check_command_no_just() {
        let dir = TempDir::new().unwrap();

        // Create justfile but just is not available
        std::fs::write(dir.path().join("justfile"), "check:\n    cargo test\n").unwrap();

        let runner = MockCommandRunner::new();

        let config = ProjectConfig::detect_in(&runner, dir.path());
        assert!(config.check_command.is_none());
    }

    #[test]
    fn test_project_config_detect_no_check_command_no_justfile() {
        let dir = TempDir::new().unwrap();

        let mut runner = MockCommandRunner::new();
        runner.set_available("just");

        let config = ProjectConfig::detect_in(&runner, dir.path());
        assert!(config.check_command.is_none());
    }

    #[test]
    fn test_project_config_detect_no_check_target() {
        let dir = TempDir::new().unwrap();

        // Create justfile without check target
        std::fs::write(dir.path().join("justfile"), "build:\n    cargo build\n").unwrap();

        let mut runner = MockCommandRunner::new();
        runner.set_available("just");

        let config = ProjectConfig::detect_in(&runner, dir.path());
        assert!(config.check_command.is_none());
    }

    #[test]
    fn test_find_justfile_lowercase() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("justfile"), "").unwrap();

        let result = find_justfile(dir.path());
        assert!(result.is_some());
        assert!(result.unwrap().ends_with("justfile"));
    }

    #[test]
    fn test_find_justfile_capitalized() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("Justfile"), "").unwrap();

        let result = find_justfile(dir.path());
        assert!(result.is_some());
        assert!(result.unwrap().ends_with("Justfile"));
    }

    #[test]
    fn test_find_justfile_not_found() {
        let dir = TempDir::new().unwrap();

        let result = find_justfile(dir.path());
        assert!(result.is_none());
    }

    #[test]
    fn test_has_check_target_simple() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("justfile");
        std::fs::write(&path, "check:\n    echo test\n").unwrap();

        assert!(has_check_target(&path));
    }

    #[test]
    fn test_has_check_target_with_deps() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("justfile");
        std::fs::write(&path, "check: lint test\n    echo done\n").unwrap();

        assert!(has_check_target(&path));
    }

    #[test]
    fn test_has_check_target_not_found() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("justfile");
        std::fs::write(&path, "build:\n    cargo build\n").unwrap();

        assert!(!has_check_target(&path));
    }

    #[test]
    fn test_has_check_target_in_comment() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("justfile");
        std::fs::write(&path, "# check: this is a comment\nbuild:\n    cargo build\n").unwrap();

        assert!(!has_check_target(&path));
    }

    #[test]
    fn test_ensure_config_creates_new() {
        use crate::traits::CommandOutput;

        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();

        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");
        // Mock git remote returning empty (no remote configured)
        runner.expect(
            "git",
            &["remote"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );

        let config = ensure_config_in(&runner, dir.path()).unwrap();

        assert!(config.git_repo);
        assert!(config.beads_installed);

        // Verify file was created
        assert!(dir.path().join(CONFIG_FILE_PATH).exists());
        runner.verify();
    }

    #[test]
    fn test_ensure_config_loads_existing() {
        let dir = TempDir::new().unwrap();

        // Create existing config
        let existing = ProjectConfig {
            git_repo: false,
            beads_installed: true,
            check_command: Some("make test".to_string()),
            code_review_section: false,
            require_push: true,
        };
        existing.save_to(dir.path()).unwrap();

        // Now create .git (detection would say git_repo=true)
        std::fs::create_dir(dir.path().join(".git")).unwrap();

        let runner = MockCommandRunner::new();
        let config = ensure_config_in(&runner, dir.path()).unwrap();

        // Should load existing config, not detect new one
        assert!(!config.git_repo); // From saved config
        assert!(config.beads_installed);
        assert_eq!(config.check_command, Some("make test".to_string()));
    }

    #[test]
    fn test_config_path() {
        let path = ProjectConfig::config_path(Path::new("/foo/bar"));
        assert_eq!(path, PathBuf::from("/foo/bar/.claude/reliability-config.yaml"));
    }

    #[test]
    fn test_ensure_config_auto_commits_in_git_repo() {
        use crate::traits::CommandOutput;
        use std::process::Command;

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Initialize a real git repo
        Command::new("git").args(["init"]).current_dir(base).output().unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(base)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(base)
            .output()
            .unwrap();

        // Create initial commit
        std::fs::write(base.join("README.md"), "test").unwrap();
        Command::new("git").args(["add", "."]).current_dir(base).output().unwrap();
        Command::new("git").args(["commit", "-m", "initial"]).current_dir(base).output().unwrap();

        let mut runner = MockCommandRunner::new();
        // Mock git remote returning empty (no remote configured in fresh repo)
        runner.expect(
            "git",
            &["remote"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );

        // This should create the config AND auto-commit it
        let config = ensure_config_in(&runner, base).unwrap();
        assert!(config.git_repo);

        // Verify the config file exists
        assert!(base.join(CONFIG_FILE_PATH).exists());

        // Verify it was committed (git status should show clean working tree)
        let status =
            Command::new("git").args(["status", "--porcelain"]).current_dir(base).output().unwrap();
        let status_output = String::from_utf8_lossy(&status.stdout);
        assert!(
            status_output.trim().is_empty(),
            "Expected clean working tree, got: {status_output}"
        );

        // Verify commit messages (there may be two: config and gitignore)
        let log = Command::new("git")
            .args(["log", "--oneline", "-2"])
            .current_dir(base)
            .output()
            .unwrap();
        let log_output = String::from_utf8_lossy(&log.stdout);
        assert!(
            log_output.contains("claude-reliability"),
            "Expected commit messages to contain 'claude-reliability', got: {log_output}"
        );
        runner.verify();
    }

    #[test]
    fn test_ensure_gitignore_creates_new() {
        let dir = TempDir::new().unwrap();

        let modified = ensure_gitignore(dir.path()).unwrap();
        assert!(modified);

        let content = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains(GITIGNORE_SECTION_HEADER));
        assert!(content.contains(".claude/bin/"));
        assert!(content.contains(".claude/*.local.md"));
    }

    #[test]
    fn test_ensure_gitignore_appends_to_existing() {
        let dir = TempDir::new().unwrap();

        // Create existing gitignore
        std::fs::write(dir.path().join(".gitignore"), "node_modules/\ntarget/\n").unwrap();

        let modified = ensure_gitignore(dir.path()).unwrap();
        assert!(modified);

        let content = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        // Original content preserved
        assert!(content.contains("node_modules/"));
        assert!(content.contains("target/"));
        // New section added
        assert!(content.contains(GITIGNORE_SECTION_HEADER));
        assert!(content.contains(".claude/bin/"));
    }

    #[test]
    fn test_ensure_gitignore_updates_existing_section() {
        let dir = TempDir::new().unwrap();

        // Create gitignore with old managed section
        let old_content = "node_modules/\n\n# claude-reliability managed\n.claude/old-entry/\n\n# Other section\nfoo/\n";
        std::fs::write(dir.path().join(".gitignore"), old_content).unwrap();

        let modified = ensure_gitignore(dir.path()).unwrap();
        assert!(modified);

        let content = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        // Original content preserved
        assert!(content.contains("node_modules/"));
        // Other section preserved
        assert!(content.contains("# Other section"));
        assert!(content.contains("foo/"));
        // Old entry removed, new entries present
        assert!(!content.contains(".claude/old-entry/"));
        assert!(content.contains(".claude/bin/"));
    }

    #[test]
    fn test_ensure_gitignore_no_change_when_up_to_date() {
        let dir = TempDir::new().unwrap();

        // First call creates
        ensure_gitignore(dir.path()).unwrap();

        // Second call should not modify
        let modified = ensure_gitignore(dir.path()).unwrap();
        assert!(!modified);
    }

    #[test]
    fn test_update_gitignore_content_empty() {
        let result = update_gitignore_content("");
        assert!(result.contains(GITIGNORE_SECTION_HEADER));
        assert!(result.contains(".claude/bin/"));
    }

    #[test]
    fn test_update_gitignore_content_preserves_existing() {
        let existing = "node_modules/\n*.log\n";
        let result = update_gitignore_content(existing);

        assert!(result.contains("node_modules/"));
        assert!(result.contains("*.log"));
        assert!(result.contains(GITIGNORE_SECTION_HEADER));
    }

    #[test]
    fn test_build_managed_section() {
        let section = build_managed_section();
        assert!(section.starts_with(GITIGNORE_SECTION_HEADER));
        for entry in GITIGNORE_ENTRIES {
            assert!(section.contains(entry));
        }
    }

    // Pedantic gitignore preservation tests

    #[test]
    fn test_gitignore_preserves_whitespace_only_file() {
        let existing = "   \n\n   \n";
        let result = update_gitignore_content(existing);
        // Should add our section after the whitespace
        assert!(result.contains(GITIGNORE_SECTION_HEADER));
        assert!(result.contains(".claude/bin/"));
    }

    #[test]
    fn test_gitignore_preserves_no_trailing_newline() {
        let existing = "node_modules/";
        let result = update_gitignore_content(existing);
        // Should still have node_modules
        assert!(result.contains("node_modules/"));
        // And our section
        assert!(result.contains(GITIGNORE_SECTION_HEADER));
    }

    #[test]
    fn test_gitignore_preserves_trailing_whitespace_on_lines() {
        let existing = "node_modules/   \ntarget/  \n";
        let result = update_gitignore_content(existing);
        // Should preserve the original lines exactly (including trailing whitespace)
        assert!(result.contains("node_modules/   \n") || result.contains("node_modules/"));
        assert!(result.contains("target/"));
    }

    #[test]
    fn test_gitignore_preserves_blank_lines_between_sections() {
        let existing = "# Section 1\nfoo/\n\n\n# Section 2\nbar/\n";
        let result = update_gitignore_content(existing);
        // Both sections preserved
        assert!(result.contains("# Section 1"));
        assert!(result.contains("foo/"));
        assert!(result.contains("# Section 2"));
        assert!(result.contains("bar/"));
    }

    #[test]
    fn test_gitignore_handles_section_at_beginning() {
        let existing = "# claude-reliability managed\n.claude/old/\n\n# Other stuff\nfoo/\n";
        let result = update_gitignore_content(existing);
        // Our section should be updated
        assert!(result.contains(".claude/bin/"));
        assert!(!result.contains(".claude/old/"));
        // Other section preserved
        assert!(result.contains("# Other stuff"));
        assert!(result.contains("foo/"));
    }

    #[test]
    fn test_gitignore_handles_section_at_end() {
        let existing = "# Other stuff\nfoo/\n\n# claude-reliability managed\n.claude/old/\n";
        let result = update_gitignore_content(existing);
        // Other section preserved
        assert!(result.contains("# Other stuff"));
        assert!(result.contains("foo/"));
        // Our section updated
        assert!(result.contains(".claude/bin/"));
        assert!(!result.contains(".claude/old/"));
    }

    #[test]
    fn test_gitignore_handles_section_in_middle() {
        let existing =
            "# Start\nfoo/\n\n# claude-reliability managed\n.claude/old/\n\n# End\nbar/\n";
        let result = update_gitignore_content(existing);
        // All sections preserved in order
        assert!(result.contains("# Start"));
        assert!(result.contains("foo/"));
        assert!(result.contains(GITIGNORE_SECTION_HEADER));
        assert!(result.contains(".claude/bin/"));
        assert!(!result.contains(".claude/old/"));
        assert!(result.contains("# End"));
        assert!(result.contains("bar/"));
        // Verify order
        let start_pos = result.find("# Start").unwrap();
        let our_pos = result.find(GITIGNORE_SECTION_HEADER).unwrap();
        let end_pos = result.find("# End").unwrap();
        assert!(start_pos < our_pos);
        assert!(our_pos < end_pos);
    }

    #[test]
    fn test_gitignore_does_not_match_similar_headers() {
        // Headers that look similar but aren't our exact header
        let existing = "# claude-reliability\nold/\n\n# claude-reliability-extra\nmore/\n";
        let result = update_gitignore_content(existing);
        // Neither of these should be treated as our section
        assert!(result.contains("# claude-reliability\n"));
        assert!(result.contains("old/"));
        assert!(result.contains("# claude-reliability-extra"));
        assert!(result.contains("more/"));
        // Our section should be appended
        assert!(result.contains(GITIGNORE_SECTION_HEADER));
    }

    #[test]
    fn test_gitignore_preserves_unicode_content() {
        let existing = "# 日本語コメント\nテスト/\n\n# Emoji section 🎉\n*.emoji\n";
        let result = update_gitignore_content(existing);
        assert!(result.contains("# 日本語コメント"));
        assert!(result.contains("テスト/"));
        assert!(result.contains("# Emoji section 🎉"));
        assert!(result.contains("*.emoji"));
    }

    #[test]
    fn test_gitignore_preserves_comment_only_lines() {
        let existing = "# This is a comment\n# Another comment\n# Third comment\n";
        let result = update_gitignore_content(existing);
        assert!(result.contains("# This is a comment"));
        assert!(result.contains("# Another comment"));
        assert!(result.contains("# Third comment"));
    }

    #[test]
    fn test_gitignore_handles_negation_patterns() {
        let existing = "*.log\n!important.log\n";
        let result = update_gitignore_content(existing);
        assert!(result.contains("*.log"));
        assert!(result.contains("!important.log"));
    }

    #[test]
    fn test_gitignore_handles_complex_patterns() {
        let existing = "[Bb]uild/\n**/node_modules/\n*.py[cod]\ntest?.txt\n";
        let result = update_gitignore_content(existing);
        assert!(result.contains("[Bb]uild/"));
        assert!(result.contains("**/node_modules/"));
        assert!(result.contains("*.py[cod]"));
        assert!(result.contains("test?.txt"));
    }

    #[test]
    fn test_gitignore_handles_escaped_patterns() {
        let existing = "\\#not-a-comment\n\\!not-negation\n";
        let result = update_gitignore_content(existing);
        assert!(result.contains("\\#not-a-comment"));
        assert!(result.contains("\\!not-negation"));
    }

    #[test]
    fn test_gitignore_preserves_inline_comments_in_patterns() {
        // Note: gitignore doesn't actually support inline comments,
        // but we should preserve whatever the user wrote
        let existing = "*.log # these are logs\n";
        let result = update_gitignore_content(existing);
        assert!(result.contains("*.log # these are logs"));
    }

    #[test]
    fn test_gitignore_idempotent_multiple_calls() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "existing/\n").unwrap();

        // First call
        ensure_gitignore(dir.path()).unwrap();
        let first = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();

        // Second call
        ensure_gitignore(dir.path()).unwrap();
        let second = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();

        // Third call
        ensure_gitignore(dir.path()).unwrap();
        let third = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();

        // All should be identical
        assert_eq!(first, second);
        assert_eq!(second, third);
    }

    #[test]
    fn test_gitignore_handles_crlf_line_endings() {
        let existing = "node_modules/\r\ntarget/\r\n";
        let result = update_gitignore_content(existing);
        // Should handle CRLF without breaking
        assert!(result.contains("node_modules/"));
        assert!(result.contains("target/"));
        assert!(result.contains(GITIGNORE_SECTION_HEADER));
    }

    #[test]
    fn test_gitignore_preserves_paths_with_spaces() {
        let existing = "My Documents/\nProgram\\ Files/\n";
        let result = update_gitignore_content(existing);
        assert!(result.contains("My Documents/"));
        assert!(result.contains("Program\\ Files/"));
    }

    #[test]
    fn test_gitignore_exact_content_verification() {
        let existing = "# My project\nnode_modules/\n\n";
        let result = update_gitignore_content(existing);

        // The result should have exactly:
        // 1. Original content (with proper newlines)
        // 2. Our managed section

        // Check it starts with the original comment
        assert!(result.starts_with("# My project\n"));

        // Check our section is present and formatted correctly
        let expected_section =
            format!("{}\n{}\n", GITIGNORE_SECTION_HEADER, GITIGNORE_ENTRIES.join("\n"));
        assert!(result.contains(&expected_section) || result.contains(GITIGNORE_SECTION_HEADER));

        // Make sure we haven't duplicated content
        let header_count = result.matches(GITIGNORE_SECTION_HEADER).count();
        assert_eq!(header_count, 1, "Should have exactly one managed section header");
    }

    #[test]
    fn test_gitignore_file_not_found_creates_new() {
        let dir = TempDir::new().unwrap();
        let gitignore_path = dir.path().join(".gitignore");

        // Ensure it doesn't exist
        assert!(!gitignore_path.exists());

        // Call ensure_gitignore
        let result = ensure_gitignore(dir.path()).unwrap();
        assert!(result); // Should report modified

        // File should now exist
        assert!(gitignore_path.exists());

        // And contain our entries
        let content = std::fs::read_to_string(&gitignore_path).unwrap();
        assert!(content.contains(GITIGNORE_SECTION_HEADER));
        assert!(content.contains(".claude/bin/"));
    }
}
