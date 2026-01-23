//! Path utilities for determining data storage locations.
//!
//! This module provides functions to determine where claude-reliability
//! stores its data files. Data is stored in `<project>/.claude-reliability/`
//! which keeps all plugin data within the project directory.

use std::path::{Path, PathBuf};

/// The data directory name within a project.
const DATA_DIR_NAME: &str = ".claude-reliability";

/// The database filename.
pub const DATABASE_FILENAME: &str = "working-memory.sqlite3";

/// Get the project-specific data directory.
///
/// Returns `<project_dir>/.claude-reliability/`.
///
/// # Arguments
///
/// * `project_dir` - The project directory to get data dir for.
#[must_use]
pub fn project_data_dir(project_dir: &Path) -> PathBuf {
    project_dir.join(DATA_DIR_NAME)
}

/// Get the database path for a project.
///
/// Returns `<project_dir>/.claude-reliability/working-memory.sqlite3`.
///
/// # Arguments
///
/// * `project_dir` - The project directory to get database path for.
#[must_use]
pub fn project_db_path(project_dir: &Path) -> PathBuf {
    project_data_dir(project_dir).join(DATABASE_FILENAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_data_dir_is_within_project() {
        let project = PathBuf::from("/some/project/path");
        let dir = project_data_dir(&project);
        assert_eq!(dir, PathBuf::from("/some/project/path/.claude-reliability"));
    }

    #[test]
    fn test_project_db_path_ends_with_filename() {
        let project = PathBuf::from("/some/project/path");
        let path = project_db_path(&project);
        assert_eq!(
            path,
            PathBuf::from("/some/project/path/.claude-reliability/working-memory.sqlite3")
        );
    }
}
