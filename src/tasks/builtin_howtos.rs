//! Built-in how-to guides that are baked into the binary.
//!
//! These how-tos are automatically synced to the database on first access,
//! with version tracking to ensure updates are applied when the binary is updated.

use crate::error::Result;
use rusqlite::Connection;

/// Current version of built-in how-tos.
/// Increment this when any built-in how-to content changes.
pub const BUILTIN_HOWTOS_VERSION: u32 = 1;

/// A built-in how-to definition.
pub struct BuiltinHowTo {
    /// Stable identifier for this how-to (used for updates).
    pub id: &'static str,
    /// Title of the how-to.
    pub title: &'static str,
    /// Instructions content.
    pub instructions: &'static str,
}

/// All built-in how-tos.
///
/// NOTE: Built-in how-tos have been replaced with skills in .claude-plugin/skills/.
/// Skills are more discoverable and can be invoked by name.
/// This list is intentionally empty - see skills for guidance content.
pub static BUILTIN_HOWTOS: &[BuiltinHowTo] = &[];

/// Sync built-in how-tos to the database if needed.
///
/// This function checks the stored version and only updates if the binary
/// has a newer version of the built-in how-tos.
///
/// NOTE: The `BUILTIN_HOWTOS` list is intentionally empty - guidance content
/// has been moved to skills in .claude-plugin/skills/. This function now only
/// tracks the version to prevent re-running on every startup.
///
/// # Errors
///
/// Returns an error if database operations fail.
pub fn sync_builtin_howtos(conn: &Connection) -> Result<()> {
    // Check current version (stored as text, parsed as u32)
    let stored_version: Option<u32> = conn
        .query_row("SELECT value FROM metadata WHERE key = 'builtin_howtos_version'", [], |row| {
            let val: String = row.get(0)?;
            Ok(val.parse::<u32>().ok())
        })
        .ok()
        .flatten();

    // Skip if already at current version
    if stored_version == Some(BUILTIN_HOWTOS_VERSION) {
        return Ok(());
    }

    // NOTE: BUILTIN_HOWTOS is intentionally empty - guidance is now in skills.
    // We still update the version to track that we've run.

    // Update stored version
    conn.execute(
        "INSERT OR REPLACE INTO metadata (key, value) VALUES ('builtin_howtos_version', ?1)",
        [BUILTIN_HOWTOS_VERSION],
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();

        // Create minimal schema for testing
        conn.execute_batch(
            r"
            CREATE TABLE metadata (
                key TEXT PRIMARY KEY,
                value TEXT
            );
            CREATE TABLE howtos (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                instructions TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            ",
        )
        .unwrap();

        conn
    }

    #[test]
    fn test_sync_with_empty_builtins() {
        let conn = setup_test_db();

        // Initially empty
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM howtos", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);

        // Sync (with empty BUILTIN_HOWTOS list)
        sync_builtin_howtos(&conn).unwrap();

        // Should still be empty
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM howtos", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);

        // Version should be stored
        let version: String = conn
            .query_row("SELECT value FROM metadata WHERE key = 'builtin_howtos_version'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(version, BUILTIN_HOWTOS_VERSION.to_string());
    }

    #[test]
    fn test_sync_is_idempotent() {
        let conn = setup_test_db();

        // Sync twice
        sync_builtin_howtos(&conn).unwrap();
        sync_builtin_howtos(&conn).unwrap();

        // Should still have same count (zero since BUILTIN_HOWTOS is empty)
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM howtos", [], |r| r.get(0)).unwrap();
        assert_eq!(count, i64::try_from(BUILTIN_HOWTOS.len()).unwrap());
    }

    #[test]
    fn test_builtin_howtos_list_is_empty() {
        // Verify that built-in how-tos have been replaced with skills
        assert!(BUILTIN_HOWTOS.is_empty(), "Built-in how-tos should be empty - use skills instead");
    }
}
