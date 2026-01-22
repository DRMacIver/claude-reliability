//! Path utilities for determining data storage locations.
//!
//! This module provides functions to determine where claude-reliability
//! stores its data files. Data is stored in `~/.claude-reliability/` with
//! project-specific subdirectories based on a hash of the project path.

use std::path::{Path, PathBuf};

/// The base directory name for claude-reliability data.
const DATA_DIR_NAME: &str = ".claude-reliability";

/// The database filename.
pub const DATABASE_FILENAME: &str = "working-memory.sqlite3";

/// Get the base data directory for claude-reliability.
///
/// Returns `~/.claude-reliability/` or `None` if the home directory
/// cannot be determined.
#[must_use]
pub fn data_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(DATA_DIR_NAME))
}

/// Get the project-specific data directory.
///
/// Returns `~/.claude-reliability/projects/<sanitized-path>/` where `<sanitized-path>` is
/// the canonical project path with `/` replaced by `-`.
///
/// # Arguments
///
/// * `project_dir` - The project directory to get data dir for.
///
/// # Returns
///
/// Returns `None` if the home directory cannot be determined.
#[must_use]
pub fn project_data_dir(project_dir: &Path) -> Option<PathBuf> {
    let base = data_dir()?;
    let sanitized = sanitize_path(project_dir);
    Some(base.join("projects").join(sanitized))
}

/// Get the database path for a project.
///
/// Returns `~/.claude-reliability/projects/<hash>/working-memory.sqlite3`.
///
/// # Arguments
///
/// * `project_dir` - The project directory to get database path for.
///
/// # Returns
///
/// Returns `None` if the home directory cannot be determined.
#[must_use]
pub fn project_db_path(project_dir: &Path) -> Option<PathBuf> {
    project_data_dir(project_dir).map(|dir| dir.join(DATABASE_FILENAME))
}

/// Sanitize a path for use as a directory name.
///
/// Uses the canonical path if available, falling back to the provided path.
/// Replaces `/` with `-` and removes leading `-`.
fn sanitize_path(project_dir: &Path) -> String {
    // Use canonical path if available for consistency
    let path_to_sanitize = project_dir.canonicalize().unwrap_or_else(|_| project_dir.to_path_buf());

    // Convert to string and replace / with -
    let path_str = path_to_sanitize.to_string_lossy();
    let sanitized = path_str.replace('/', "-");

    // Remove leading dash if present (from leading /)
    sanitized.trim_start_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_data_dir_returns_home_based_path() {
        if let Some(home) = dirs::home_dir() {
            let data = data_dir().unwrap();
            assert_eq!(data, home.join(".claude-reliability"));
        }
    }

    #[test]
    fn test_project_data_dir_includes_sanitized_path() {
        let project = PathBuf::from("/some/project/path");
        if let Some(dir) = project_data_dir(&project) {
            assert!(dir.to_string_lossy().contains("projects"));
            // Directory name should be sanitized path
            let dir_name = dir.file_name().unwrap().to_string_lossy();
            assert!(dir_name.contains("some-project-path"));
        }
    }

    #[test]
    fn test_project_db_path_ends_with_filename() {
        let project = PathBuf::from("/some/project/path");
        if let Some(path) = project_db_path(&project) {
            assert!(path.to_string_lossy().ends_with(DATABASE_FILENAME));
        }
    }

    #[test]
    fn test_sanitize_path_replaces_slashes() {
        let path = PathBuf::from("/home/user/project");
        let sanitized = sanitize_path(&path);
        assert!(!sanitized.contains('/'));
        assert!(sanitized.contains("home-user-project"));
    }

    #[test]
    fn test_sanitize_path_removes_leading_dash() {
        let path = PathBuf::from("/leading/slash");
        let sanitized = sanitize_path(&path);
        assert!(!sanitized.starts_with('-'));
    }

    #[test]
    fn test_sanitize_path_is_consistent() {
        let project = PathBuf::from("/consistent/path");
        let sanitized1 = sanitize_path(&project);
        let sanitized2 = sanitize_path(&project);
        assert_eq!(sanitized1, sanitized2);
    }

    #[test]
    fn test_sanitize_path_differs_for_different_paths() {
        let project1 = PathBuf::from("/path/one");
        let project2 = PathBuf::from("/path/two");
        let sanitized1 = sanitize_path(&project1);
        let sanitized2 = sanitize_path(&project2);
        assert_ne!(sanitized1, sanitized2);
    }
}
