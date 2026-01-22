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
/// Add new how-tos here and increment `BUILTIN_HOWTOS_VERSION`.
pub static BUILTIN_HOWTOS: &[BuiltinHowTo] = &[BuiltinHowTo {
    id: "builtin-using-claude-reliability-tools",
    title: "How to Use Claude Reliability Tools",
    instructions: include_str!("../../templates/howtos/using_claude_reliability_tools.md"),
}];

/// Sync built-in how-tos to the database if needed.
///
/// This function checks the stored version and only updates if the binary
/// has a newer version of the built-in how-tos.
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

    // Sync each built-in how-to
    for howto in BUILTIN_HOWTOS {
        // Check if it already exists
        let exists: bool = conn
            .query_row("SELECT 1 FROM howtos WHERE id = ?1", [howto.id], |_| Ok(true))
            .unwrap_or(false);

        if exists {
            // Update existing
            conn.execute(
                "UPDATE howtos SET title = ?1, instructions = ?2, updated_at = datetime('now') WHERE id = ?3",
                (howto.title, howto.instructions, howto.id),
            )?;
        } else {
            // Insert new
            conn.execute(
                "INSERT INTO howtos (id, title, instructions) VALUES (?1, ?2, ?3)",
                (howto.id, howto.title, howto.instructions),
            )?;
        }
    }

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
    fn test_sync_creates_builtin_howtos() {
        let conn = setup_test_db();

        // Initially empty
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM howtos", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);

        // Sync
        sync_builtin_howtos(&conn).unwrap();

        // Should have created how-tos
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM howtos", [], |r| r.get(0)).unwrap();
        assert_eq!(count, i64::try_from(BUILTIN_HOWTOS.len()).unwrap());

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

        // Should still have same count
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM howtos", [], |r| r.get(0)).unwrap();
        assert_eq!(count, i64::try_from(BUILTIN_HOWTOS.len()).unwrap());
    }

    #[test]
    fn test_sync_updates_on_version_change() {
        let conn = setup_test_db();

        // Sync at "old" version
        conn.execute("INSERT INTO metadata (key, value) VALUES ('builtin_howtos_version', 0)", [])
            .unwrap();

        // Manually insert an outdated how-to
        conn.execute(
            "INSERT INTO howtos (id, title, instructions) VALUES ('builtin-using-claude-reliability-tools', 'Old Title', 'Old instructions')",
            [],
        )
        .unwrap();

        // Sync should update
        sync_builtin_howtos(&conn).unwrap();

        // Title should be updated
        let title: String = conn
            .query_row(
                "SELECT title FROM howtos WHERE id = 'builtin-using-claude-reliability-tools'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(title, "How to Use Claude Reliability Tools");

        // Version should be updated
        let version: String = conn
            .query_row("SELECT value FROM metadata WHERE key = 'builtin_howtos_version'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(version, BUILTIN_HOWTOS_VERSION.to_string());
    }

    #[test]
    fn test_builtin_howto_content_is_valid() {
        // Verify all built-in how-tos have non-empty content
        for howto in BUILTIN_HOWTOS {
            assert!(!howto.id.is_empty(), "How-to ID should not be empty");
            assert!(!howto.title.is_empty(), "How-to title should not be empty");
            assert!(!howto.instructions.is_empty(), "How-to instructions should not be empty");
            assert!(
                howto.id.starts_with("builtin-"),
                "Built-in how-to IDs should start with 'builtin-'"
            );
        }
    }
}
