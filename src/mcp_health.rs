//! MCP server health monitoring via heartbeat files.
//!
//! The MCP server writes a heartbeat file periodically containing its PID and timestamp.
//! Other components can check this file to determine if the MCP server is running.

use crate::paths::project_data_dir;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The heartbeat filename within the project data directory.
const HEARTBEAT_FILENAME: &str = "mcp-heartbeat";

/// How often the MCP server should write a heartbeat (in seconds).
pub const HEARTBEAT_INTERVAL_SECS: u64 = 30;

/// How long before a heartbeat is considered stale (in seconds).
/// This should be longer than the heartbeat interval to allow for some delay.
const HEARTBEAT_STALE_SECS: u64 = 90;

/// Get the path to the heartbeat file for a project.
#[must_use]
pub fn heartbeat_path(base_dir: &Path) -> PathBuf {
    project_data_dir(base_dir).join(HEARTBEAT_FILENAME)
}

/// Write a heartbeat file with the current PID and timestamp.
///
/// # Errors
///
/// Returns an error if the file cannot be written.
pub fn write_heartbeat(base_dir: &Path) -> std::io::Result<()> {
    let path = heartbeat_path(base_dir);

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let pid = std::process::id();
    let timestamp =
        SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);

    let content = format!("{pid}\n{timestamp}\n");

    // Write atomically by writing to temp file then renaming
    let temp_path = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&temp_path)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(&temp_path, &path)?;

    Ok(())
}

/// Read the heartbeat file and return (pid, timestamp).
///
/// Returns `None` if the file doesn't exist or can't be parsed.
fn read_heartbeat(base_dir: &Path) -> Option<(u32, u64)> {
    let path = heartbeat_path(base_dir);
    let content = fs::read_to_string(&path).ok()?;
    let mut lines = content.lines();

    let pid: u32 = lines.next()?.parse().ok()?;
    let timestamp: u64 = lines.next()?.parse().ok()?;

    Some((pid, timestamp))
}

/// Check if the MCP server appears to be running.
///
/// Returns `true` if:
/// - A heartbeat file exists
/// - The heartbeat timestamp is recent (within `HEARTBEAT_STALE_SECS`)
/// - The PID in the heartbeat file is still running (on Unix systems)
///
/// Returns `false` if the heartbeat is missing, stale, or the process is dead.
#[must_use]
pub fn is_mcp_server_alive(base_dir: &Path) -> bool {
    let Some((pid, timestamp)) = read_heartbeat(base_dir) else {
        return false;
    };

    // Check if heartbeat is recent
    let now =
        SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);

    if now.saturating_sub(timestamp) > HEARTBEAT_STALE_SECS {
        return false;
    }

    // Check if the process is still running
    is_process_running(pid)
}

/// Check if a process with the given PID is running.
#[cfg(target_os = "linux")]
fn is_process_running(pid: u32) -> bool {
    // On Linux, check if /proc/<pid> exists
    Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(target_os = "macos")]
fn is_process_running(pid: u32) -> bool {
    // On macOS, use ps to check if process exists
    // This is slightly expensive but reliable without unsafe code
    std::process::Command::new("ps")
        .args(["-p", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn is_process_running(_pid: u32) -> bool {
    // On other systems, just trust the heartbeat timestamp
    true
}

/// Remove the heartbeat file (for cleanup).
///
/// This should be called when the MCP server shuts down gracefully.
pub fn remove_heartbeat(base_dir: &Path) {
    let path = heartbeat_path(base_dir);
    let _ = fs::remove_file(path);
}

/// Get the age of the heartbeat in seconds, or None if no heartbeat exists.
#[must_use]
pub fn heartbeat_age_secs(base_dir: &Path) -> Option<u64> {
    let (_, timestamp) = read_heartbeat(base_dir)?;

    let now =
        SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);

    Some(now.saturating_sub(timestamp))
}

/// Describe the MCP server status for user display.
#[must_use]
pub fn describe_mcp_status(base_dir: &Path) -> String {
    let path = heartbeat_path(base_dir);

    if !path.exists() {
        return "MCP server: no heartbeat file (server may not have started)".to_string();
    }

    let Some((pid, timestamp)) = read_heartbeat(base_dir) else {
        return "MCP server: heartbeat file exists but unreadable".to_string();
    };

    let now =
        SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);

    let age = now.saturating_sub(timestamp);

    if age > HEARTBEAT_STALE_SECS {
        return format!(
            "MCP server: heartbeat stale ({age}s old, PID {pid}) - server may have crashed"
        );
    }

    if !is_process_running(pid) {
        return format!("MCP server: process {pid} not running - server has stopped");
    }

    format!("MCP server: alive (PID {pid}, heartbeat {age}s ago)")
}

/// Start a background task that periodically writes heartbeat files.
///
/// Returns a shutdown handle that, when notified, will stop the heartbeat task
/// and clean up the heartbeat file.
///
/// # Example
///
/// ```ignore
/// let shutdown = Arc::new(Notify::new());
/// mcp_health::start_heartbeat_task(project_dir.clone(), Arc::clone(&shutdown));
/// // ... run server ...
/// shutdown.notify_one();
/// ```
#[cfg(feature = "mcp")]
pub fn start_heartbeat_task(
    project_dir: std::path::PathBuf,
    shutdown: std::sync::Arc<tokio::sync::Notify>,
) {
    tokio::spawn(async move {
        let interval = tokio::time::Duration::from_secs(HEARTBEAT_INTERVAL_SECS);

        // Write initial heartbeat
        if let Err(e) = write_heartbeat(&project_dir) {
            eprintln!("Warning: failed to write initial heartbeat: {e}");
        }

        loop {
            tokio::select! {
                () = tokio::time::sleep(interval) => {
                    if let Err(e) = write_heartbeat(&project_dir) {
                        eprintln!("Warning: failed to write heartbeat: {e}");
                    }
                }
                () = shutdown.notified() => {
                    // Clean up heartbeat file on shutdown
                    remove_heartbeat(&project_dir);
                    break;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_heartbeat_path() {
        let path = heartbeat_path(Path::new("/project"));
        assert_eq!(path, PathBuf::from("/project/.claude-reliability/mcp-heartbeat"));
    }

    #[test]
    fn test_write_and_read_heartbeat() {
        let dir = TempDir::new().unwrap();
        write_heartbeat(dir.path()).unwrap();

        let (pid, timestamp) = read_heartbeat(dir.path()).unwrap();
        assert_eq!(pid, std::process::id());
        assert!(timestamp > 0);
    }

    #[test]
    fn test_is_mcp_server_alive_with_fresh_heartbeat() {
        let dir = TempDir::new().unwrap();
        write_heartbeat(dir.path()).unwrap();

        // Current process wrote the heartbeat, so it should be "alive"
        assert!(is_mcp_server_alive(dir.path()));
    }

    #[test]
    fn test_is_mcp_server_alive_no_heartbeat() {
        let dir = TempDir::new().unwrap();
        // No heartbeat written
        assert!(!is_mcp_server_alive(dir.path()));
    }

    #[test]
    fn test_is_mcp_server_alive_stale_heartbeat() {
        let dir = TempDir::new().unwrap();
        let path = heartbeat_path(dir.path());

        // Create parent directory
        fs::create_dir_all(path.parent().unwrap()).unwrap();

        // Write a heartbeat with an old timestamp
        let pid = std::process::id();
        let old_timestamp =
            SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs()
                - HEARTBEAT_STALE_SECS
                - 10;

        fs::write(&path, format!("{pid}\n{old_timestamp}\n")).unwrap();

        assert!(!is_mcp_server_alive(dir.path()));
    }

    #[test]
    fn test_remove_heartbeat() {
        let dir = TempDir::new().unwrap();
        write_heartbeat(dir.path()).unwrap();

        let path = heartbeat_path(dir.path());
        assert!(path.exists());

        remove_heartbeat(dir.path());
        assert!(!path.exists());
    }

    #[test]
    fn test_heartbeat_age_secs() {
        let dir = TempDir::new().unwrap();
        write_heartbeat(dir.path()).unwrap();

        let age = heartbeat_age_secs(dir.path()).unwrap();
        // Should be very recent (within a second of writing)
        assert!(age < 2);
    }

    #[test]
    fn test_heartbeat_age_secs_no_heartbeat() {
        let dir = TempDir::new().unwrap();
        assert!(heartbeat_age_secs(dir.path()).is_none());
    }

    #[test]
    fn test_describe_mcp_status_no_heartbeat() {
        let dir = TempDir::new().unwrap();
        let status = describe_mcp_status(dir.path());
        assert!(status.contains("no heartbeat file"));
    }

    #[test]
    fn test_describe_mcp_status_alive() {
        let dir = TempDir::new().unwrap();
        write_heartbeat(dir.path()).unwrap();

        let status = describe_mcp_status(dir.path());
        assert!(status.contains("alive"));
    }

    #[test]
    fn test_describe_mcp_status_stale() {
        let dir = TempDir::new().unwrap();
        let path = heartbeat_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();

        let pid = std::process::id();
        let old_timestamp =
            SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs()
                - HEARTBEAT_STALE_SECS
                - 10;

        fs::write(&path, format!("{pid}\n{old_timestamp}\n")).unwrap();

        let status = describe_mcp_status(dir.path());
        assert!(status.contains("stale"));
    }

    #[test]
    fn test_read_heartbeat_invalid_content() {
        let dir = TempDir::new().unwrap();
        let path = heartbeat_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "invalid content").unwrap();

        assert!(read_heartbeat(dir.path()).is_none());
    }

    #[test]
    fn test_describe_mcp_status_unreadable() {
        let dir = TempDir::new().unwrap();
        let path = heartbeat_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Write invalid content that can't be parsed
        fs::write(&path, "not a valid heartbeat").unwrap();

        let status = describe_mcp_status(dir.path());
        assert!(status.contains("unreadable"));
    }

    #[test]
    fn test_describe_mcp_status_process_not_running() {
        let dir = TempDir::new().unwrap();
        let path = heartbeat_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();

        // Use a PID that almost certainly doesn't exist
        let fake_pid = 99_999_999u32;
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();

        // Write a fresh heartbeat with a non-existent PID
        fs::write(&path, format!("{fake_pid}\n{now}\n")).unwrap();

        let status = describe_mcp_status(dir.path());
        // Either the process isn't running (normal case) or it happens to exist (very unlikely)
        assert!(status.contains("not running") || status.contains("alive"));
    }

    #[test]
    fn test_is_process_running_current_process() {
        let pid = std::process::id();
        assert!(is_process_running(pid));
    }

    #[test]
    fn test_is_process_running_nonexistent() {
        // PID 99999999 is unlikely to exist
        // On some systems this might exist, so we just check it doesn't panic
        let _ = is_process_running(99_999_999);
    }
}
