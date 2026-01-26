//! MCP server logging to help debug disconnections.
//!
//! Writes logs to `.claude-reliability/mcp.log` in the project directory.
//! Since stdout/stderr are captured by the MCP protocol, this logs directly to file.

use crate::paths::project_data_dir;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::panic;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Instant, SystemTime};

/// The log filename within the project data directory.
const LOG_FILENAME: &str = "mcp.log";

/// Maximum log file size before rotation (1MB).
const MAX_LOG_SIZE: u64 = 1_048_576;

/// Global log file handle (lazily initialized).
static LOG_FILE: Mutex<Option<File>> = Mutex::new(None);

/// Global project directory for logging.
static PROJECT_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Get the path to the MCP log file.
#[must_use]
pub fn log_path(base_dir: &Path) -> PathBuf {
    project_data_dir(base_dir).join(LOG_FILENAME)
}

/// Initialize the MCP logger for a project directory.
///
/// This should be called once at MCP server startup.
///
/// # Errors
///
/// Returns an error if the log file cannot be created.
pub fn init(base_dir: &Path) -> std::io::Result<()> {
    let path = log_path(base_dir);

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Rotate log file if too large
    if path.exists() {
        if let Ok(metadata) = fs::metadata(&path) {
            if metadata.len() > MAX_LOG_SIZE {
                let backup = path.with_extension("log.old");
                let _ = fs::rename(&path, backup);
            }
        }
    }

    // Open log file in append mode
    let file = OpenOptions::new().create(true).append(true).open(&path)?;

    // Store in globals
    if let Ok(mut guard) = LOG_FILE.lock() {
        *guard = Some(file);
    }
    if let Ok(mut guard) = PROJECT_DIR.lock() {
        *guard = Some(base_dir.to_path_buf());
    }

    // Log startup
    log_event("MCP server starting");

    Ok(())
}

/// Format the current timestamp for logging.
fn timestamp() -> String {
    let now =
        SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);

    // Format as ISO-8601 (approximate, without timezone complexity)
    let secs_per_day = 86400;
    let secs_per_hour = 3600;
    let secs_per_min = 60;

    let days = now / secs_per_day;
    let remaining = now % secs_per_day;
    let hours = remaining / secs_per_hour;
    let remaining = remaining % secs_per_hour;
    let mins = remaining / secs_per_min;
    let secs = remaining % secs_per_min;

    // Days since epoch (1970-01-01)
    // Simple approximation - good enough for logging
    let years = days / 365;
    let year = 1970 + years;
    let day_of_year = days % 365;
    let month = (day_of_year / 30).min(11) + 1;
    let day = (day_of_year % 30) + 1;

    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{mins:02}:{secs:02}Z")
}

/// Write a log entry.
fn write_log(message: &str) {
    if let Ok(mut guard) = LOG_FILE.lock() {
        if let Some(ref mut file) = *guard {
            let ts = timestamp();
            let _ = writeln!(file, "[{ts}] {message}");
            let _ = file.flush();
        }
    }
}

/// Log a general event.
pub fn log_event(message: &str) {
    write_log(&format!("EVENT: {message}"));
}

/// Log a tool call start.
pub fn log_tool_start(tool_name: &str) {
    write_log(&format!("TOOL_START: {tool_name}"));
}

/// Log a tool call completion with duration.
pub fn log_tool_end(tool_name: &str, duration_ms: u128, success: bool) {
    let status = if success { "OK" } else { "ERROR" };
    write_log(&format!("TOOL_END: {tool_name} ({duration_ms}ms) [{status}]"));
}

/// Log an error.
pub fn log_error(message: &str) {
    write_log(&format!("ERROR: {message}"));
}

/// Log a warning.
pub fn log_warning(message: &str) {
    write_log(&format!("WARN: {message}"));
}

/// Log a panic with backtrace.
#[allow(deprecated)] // PanicInfo is deprecated but PanicHookInfo requires Rust 1.81+
fn log_panic(info: &panic::PanicInfo<'_>) {
    // Format location - use the location if available (always present in practice)
    let location = format_panic_location(info.location());

    let payload = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic payload".to_string());

    write_log(&format!("PANIC at {location}: {payload}"));

    // Log backtrace if available (depends on RUST_BACKTRACE env var)
    let backtrace = std::backtrace::Backtrace::capture();
    log_backtrace_str(&backtrace.to_string());
}

/// Format a panic location for logging.
fn format_panic_location(location: Option<&panic::Location<'_>>) -> String {
    location.map_or_else(
        || "unknown".to_string(),
        |loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()),
    )
}

/// Log backtrace lines from a backtrace string.
fn log_backtrace_str(bt_str: &str) {
    // Backtrace::capture() returns an empty or "disabled" message when not enabled
    if bt_str.is_empty() || bt_str.contains("disabled") {
        return;
    }

    for line in bt_str.lines() {
        write_log(&format!("  {line}"));
    }
}

/// Install a panic hook that logs panics to the MCP log file.
///
/// This should be called after `init()`.
pub fn install_panic_hook() {
    let original_hook = panic::take_hook();

    panic::set_hook(Box::new(move |info| {
        // Log to our file
        log_panic(info);

        // Also call the original hook (which may print to stderr)
        original_hook(info);
    }));

    log_event("Panic hook installed");
}

/// Log MCP server shutdown.
pub fn log_shutdown(exit_code: Option<i32>) {
    match exit_code {
        Some(code) => write_log(&format!("SHUTDOWN: exit code {code}")),
        None => write_log("SHUTDOWN: normal"),
    }
}

/// A guard that logs tool call duration when dropped.
///
/// Use this to wrap tool call execution:
/// ```ignore
/// let _guard = ToolCallGuard::new("create_work_item");
/// // ... execute tool ...
/// // guard logs duration when dropped
/// ```
pub struct ToolCallGuard {
    tool_name: String,
    start: Instant,
    success: bool,
}

impl ToolCallGuard {
    /// Create a new tool call guard and log the start.
    #[must_use]
    pub fn new(tool_name: &str) -> Self {
        log_tool_start(tool_name);
        Self { tool_name: tool_name.to_string(), start: Instant::now(), success: true }
    }

    /// Mark the tool call as failed.
    pub fn mark_error(&mut self) {
        self.success = false;
    }
}

impl Drop for ToolCallGuard {
    fn drop(&mut self) {
        let duration = self.start.elapsed().as_millis();
        log_tool_end(&self.tool_name, duration, self.success);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_log_path() {
        let path = log_path(Path::new("/project"));
        assert_eq!(path, PathBuf::from("/project/.claude-reliability/mcp.log"));
    }

    // Tests below must be serial because they share global state (LOG_FILE, PROJECT_DIR)

    #[serial_test::serial]
    #[test]
    fn test_init_creates_log_file() {
        let dir = TempDir::new().unwrap();
        init(dir.path()).unwrap();

        let path = log_path(dir.path());
        assert!(path.exists());
    }

    #[serial_test::serial]
    #[test]
    fn test_log_event_writes_to_file() {
        let dir = TempDir::new().unwrap();
        init(dir.path()).unwrap();

        log_event("test event");

        let path = log_path(dir.path());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("EVENT: test event"));
    }

    #[serial_test::serial]
    #[test]
    fn test_tool_call_guard_logs_duration() {
        let dir = TempDir::new().unwrap();
        init(dir.path()).unwrap();

        {
            let _guard = ToolCallGuard::new("test_tool");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let path = log_path(dir.path());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("TOOL_START: test_tool"));
        assert!(content.contains("TOOL_END: test_tool"));
        assert!(content.contains("[OK]"));
    }

    #[serial_test::serial]
    #[test]
    fn test_tool_call_guard_logs_error() {
        let dir = TempDir::new().unwrap();
        init(dir.path()).unwrap();

        {
            let mut guard = ToolCallGuard::new("failing_tool");
            guard.mark_error();
        }

        let path = log_path(dir.path());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("TOOL_END: failing_tool"));
        assert!(content.contains("[ERROR]"));
    }

    #[serial_test::serial]
    #[test]
    fn test_log_rotation() {
        let dir = TempDir::new().unwrap();
        let path = log_path(dir.path());

        // Create parent directory
        fs::create_dir_all(path.parent().unwrap()).unwrap();

        // Create an oversized log file
        let size = usize::try_from(MAX_LOG_SIZE + 1).unwrap();
        let large_content = "x".repeat(size);
        fs::write(&path, large_content).unwrap();

        // Init should rotate it
        init(dir.path()).unwrap();

        // Old log should exist
        let old_path = path.with_extension("log.old");
        assert!(old_path.exists());

        // New log should be small
        let metadata = fs::metadata(&path).unwrap();
        assert!(metadata.len() < MAX_LOG_SIZE);
    }

    #[serial_test::serial]
    #[test]
    fn test_log_error() {
        let dir = TempDir::new().unwrap();
        init(dir.path()).unwrap();

        log_error("test error message");

        let path = log_path(dir.path());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("ERROR: test error message"));
    }

    #[serial_test::serial]
    #[test]
    fn test_log_warning() {
        let dir = TempDir::new().unwrap();
        init(dir.path()).unwrap();

        log_warning("test warning message");

        let path = log_path(dir.path());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("WARN: test warning message"));
    }

    #[serial_test::serial]
    #[test]
    fn test_log_shutdown() {
        let dir = TempDir::new().unwrap();
        init(dir.path()).unwrap();

        log_shutdown(None);

        let path = log_path(dir.path());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("SHUTDOWN: normal"));
    }

    #[serial_test::serial]
    #[test]
    fn test_log_shutdown_with_exit_code() {
        let dir = TempDir::new().unwrap();
        init(dir.path()).unwrap();

        log_shutdown(Some(42));

        let path = log_path(dir.path());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("SHUTDOWN: exit code 42"));
    }

    #[serial_test::serial]
    #[test]
    fn test_install_panic_hook_and_panic_logging() {
        let dir = TempDir::new().unwrap();
        init(dir.path()).unwrap();
        install_panic_hook();

        // Trigger a panic in a catch_unwind to test the panic hook
        let _ = std::panic::catch_unwind(|| {
            panic!("test panic message");
        });

        let path = log_path(dir.path());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("PANIC at"));
        assert!(content.contains("test panic message"));
    }

    #[serial_test::serial]
    #[test]
    fn test_log_backtrace_str_with_content() {
        let dir = TempDir::new().unwrap();
        init(dir.path()).unwrap();

        // Test with a mock backtrace string
        log_backtrace_str("  0: test::function\n  1: another::function\n");

        let path = log_path(dir.path());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("0: test::function"));
        assert!(content.contains("1: another::function"));
    }

    #[serial_test::serial]
    #[test]
    fn test_log_backtrace_str_disabled() {
        let dir = TempDir::new().unwrap();
        init(dir.path()).unwrap();

        // Test with disabled backtrace message - should not log anything new
        let path = log_path(dir.path());
        let before_content = fs::read_to_string(&path).unwrap();

        log_backtrace_str("disabled backtrace");

        let after_content = fs::read_to_string(&path).unwrap();
        // Content should be the same (only the startup message)
        assert_eq!(before_content, after_content);
    }

    #[test]
    fn test_format_panic_location_none() {
        let result = format_panic_location(None);
        assert_eq!(result, "unknown");
    }

    #[test]
    fn test_format_panic_location_some() {
        // We can't easily create a panic::Location, but we can test the None case
        // and the Some case is implicitly tested by test_install_panic_hook_and_panic_logging
        // which triggers a real panic with a location
    }
}
