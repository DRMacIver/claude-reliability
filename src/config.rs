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

/// Project configuration for reliability hooks.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
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

        Self { git_repo, beads_installed, check_command }
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
/// added and committed.
///
/// # Errors
///
/// Returns an error if config cannot be loaded or saved.
pub fn ensure_config_in(runner: &dyn CommandRunner, base_dir: &Path) -> Result<ProjectConfig> {
    // Try to load existing config
    if let Some(config) = ProjectConfig::load_from(base_dir)? {
        return Ok(config);
    }

    // Detect and create new config
    let config = ProjectConfig::detect_in(runner, base_dir);
    config.save_to(base_dir)?;

    // Auto-commit if in a git repo
    if config.git_repo {
        auto_commit_config(base_dir);
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
    let add_result = Command::new("git")
        .args(["add", &config_path_str])
        .current_dir(base_dir)
        .output();

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
        .args(["commit", "-m", "Add claude-reliability config\n\nAuto-generated by claude-reliability plugin."])
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

        let config =
            ProjectConfig { git_repo: true, beads_installed: true, check_command: Some("just check".to_string()) };

        config.save_to(dir.path()).unwrap();

        let loaded = ProjectConfig::load_from(dir.path()).unwrap().unwrap();
        assert_eq!(config, loaded);
    }

    #[test]
    fn test_project_config_yaml_format() {
        let dir = TempDir::new().unwrap();

        let config =
            ProjectConfig { git_repo: true, beads_installed: false, check_command: Some("make test".to_string()) };

        config.save_to(dir.path()).unwrap();

        let content = std::fs::read_to_string(dir.path().join(CONFIG_FILE_PATH)).unwrap();
        assert!(content.contains("git_repo: true"));
        assert!(content.contains("beads_installed: false"));
        assert!(content.contains("check_command: make test"));
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
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        let runner = MockCommandRunner::new();

        let config = ProjectConfig::detect_in(&runner, dir.path());
        assert!(config.git_repo);
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
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();

        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");

        let config = ensure_config_in(&runner, dir.path()).unwrap();

        assert!(config.git_repo);
        assert!(config.beads_installed);

        // Verify file was created
        assert!(dir.path().join(CONFIG_FILE_PATH).exists());
    }

    #[test]
    fn test_ensure_config_loads_existing() {
        let dir = TempDir::new().unwrap();

        // Create existing config
        let existing = ProjectConfig {
            git_repo: false,
            beads_installed: true,
            check_command: Some("make test".to_string()),
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
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(base)
            .output()
            .unwrap();

        let runner = MockCommandRunner::new();

        // This should create the config AND auto-commit it
        let config = ensure_config_in(&runner, base).unwrap();
        assert!(config.git_repo);

        // Verify the config file exists
        assert!(base.join(CONFIG_FILE_PATH).exists());

        // Verify it was committed (git status should show clean working tree)
        let status = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(base)
            .output()
            .unwrap();
        let status_output = String::from_utf8_lossy(&status.stdout);
        assert!(
            status_output.trim().is_empty(),
            "Expected clean working tree, got: {status_output}"
        );

        // Verify the commit message
        let log = Command::new("git")
            .args(["log", "--oneline", "-1"])
            .current_dir(base)
            .output()
            .unwrap();
        let log_output = String::from_utf8_lossy(&log.stdout);
        assert!(
            log_output.contains("claude-reliability config"),
            "Expected commit message to contain 'claude-reliability config', got: {log_output}"
        );
    }
}
