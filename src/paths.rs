//! Path utilities for determining data storage locations.
//!
//! This module provides functions to determine where claude-reliability
//! stores its data files. Data is stored in `~/.claude-reliability/` with
//! project-specific subdirectories based on a hash of the project path.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
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
/// Returns `~/.claude-reliability/projects/<readable-hash>/` where `<readable-hash>` is
/// a human-readable prefix plus a hash suffix to avoid collisions.
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
    let dir_name = create_project_dir_name(project_dir);
    Some(base.join("projects").join(dir_name))
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

/// Create a directory name for a project.
///
/// Uses a readable prefix (last path component) plus a hash suffix to ensure
/// uniqueness while remaining human-readable.
///
/// Format: `<project-name>-<hash>` e.g., `my-project-a1b2c3d4`
fn create_project_dir_name(project_dir: &Path) -> String {
    // Use canonical path if available for consistency
    let path_to_hash = project_dir.canonicalize().unwrap_or_else(|_| project_dir.to_path_buf());

    // Get the last component as a readable prefix
    let prefix = path_to_hash.file_name().and_then(|n| n.to_str()).unwrap_or("project");

    // Sanitize the prefix (replace non-alphanumeric with dash, collapse multiple dashes)
    let prefix: String =
        prefix.chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
    let prefix = prefix.trim_matches('-');

    // Hash the full canonical path for uniqueness
    let hash = hash_path(&path_to_hash);

    format!("{prefix}-{hash:016x}")
}

/// Compute a stable hash of a path.
fn hash_path(path: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
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
    fn test_project_data_dir_includes_project_name() {
        let project = PathBuf::from("/some/project/path");
        if let Some(dir) = project_data_dir(&project) {
            assert!(dir.to_string_lossy().contains("projects"));
            // Directory name should include the project name prefix
            let dir_name = dir.file_name().unwrap().to_string_lossy();
            assert!(dir_name.starts_with("path-"));
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
    fn test_create_project_dir_name_no_slashes() {
        let path = PathBuf::from("/home/user/project");
        let dir_name = create_project_dir_name(&path);
        assert!(!dir_name.contains('/'));
    }

    #[test]
    fn test_create_project_dir_name_starts_with_project_name() {
        let path = PathBuf::from("/leading/slash");
        let dir_name = create_project_dir_name(&path);
        assert!(dir_name.starts_with("slash-"));
    }

    #[test]
    fn test_create_project_dir_name_is_consistent() {
        let project = PathBuf::from("/consistent/path");
        let dir_name1 = create_project_dir_name(&project);
        let dir_name2 = create_project_dir_name(&project);
        assert_eq!(dir_name1, dir_name2);
    }

    #[test]
    fn test_create_project_dir_name_differs_for_different_paths() {
        let project1 = PathBuf::from("/path/one");
        let project2 = PathBuf::from("/path/two");
        let dir_name1 = create_project_dir_name(&project1);
        let dir_name2 = create_project_dir_name(&project2);
        assert_ne!(dir_name1, dir_name2);
    }

    #[test]
    fn test_create_project_dir_name_no_collision_similar_paths() {
        // This was the original bug: these paths would both map to "home-user-project"
        let project1 = PathBuf::from("/home/user/project");
        let project2 = PathBuf::from("/home/user-project");
        let dir_name1 = create_project_dir_name(&project1);
        let dir_name2 = create_project_dir_name(&project2);
        // With hashing, these should be different
        assert_ne!(dir_name1, dir_name2);
    }

    #[test]
    fn test_hash_path_deterministic() {
        let path = PathBuf::from("/test/path");
        let hash1 = hash_path(&path);
        let hash2 = hash_path(&path);
        assert_eq!(hash1, hash2);
    }
}
