use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::{params, Connection};

use crate::error::{CoreError, Result};
use crate::model::ClipKind;

/// Current SQLite database schema version.
const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Busy timeout for acquiring database locks.
const BUSY_TIMEOUT: Duration = Duration::from_millis(5000);

/// Record representing a clipboard item stored in local history.
///
/// Note: Content data is stored as plaintext at rest in the current version;
/// database-level or payload-level encryption at rest is planned for a future release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    /// Unique ULID identifier of the item.
    pub id: String,
    /// Originating device identifier.
    pub origin: String,
    /// Classification of content (text or image).
    pub kind: ClipKind,
    /// MIME type of the content bytes.
    pub mime: String,
    /// BLAKE3 checksum of the clipboard content.
    pub hash: Vec<u8>,
    /// Byte length of the content `data`.
    pub size: usize,
    /// Unix millisecond timestamp when the item was created.
    pub created_at: i64,
    /// Plaintext content bytes (stored unencrypted at rest for now).
    pub data: Vec<u8>,
    /// `true` if this item was received from a remote group member, `false` if captured locally.
    pub is_remote: bool,
}

/// SQLite-backed persistent clipboard history database.
///
/// Configured with WAL journal mode and busy timeout for concurrent read/write resilience.
/// Uses PRAGMA `user_version` to track schema versions for seamless future migrations.
pub struct History {
    conn: Mutex<Connection>,
}

impl History {
    /// Opens or creates a SQLite clipboard history database at the given path.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    /// Opens an in-memory SQLite clipboard history database (primarily for testing).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.busy_timeout(BUSY_TIMEOUT)?;

        // WAL mode may fail on some memory databases, ignore error if so
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        let _ = conn.pragma_update(None, "synchronous", "NORMAL");
        let _ = conn.pragma_update(None, "foreign_keys", "ON");

        let version: u32 = conn.query_row("PRAGMA user_version;", [], |row| row.get(0))?;

        if version < CURRENT_SCHEMA_VERSION {
            conn.execute_batch(
                "
                CREATE TABLE IF NOT EXISTS history (
                    id TEXT PRIMARY KEY,
                    origin TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    mime TEXT NOT NULL,
                    hash BLOB NOT NULL,
                    size INTEGER NOT NULL,
                    created_at INTEGER NOT NULL,
                    data BLOB NOT NULL,
                    is_remote INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_history_created_at ON history(created_at DESC);
                CREATE INDEX IF NOT EXISTS idx_history_hash ON history(hash);
                CREATE INDEX IF NOT EXISTS idx_history_is_remote_id ON history(is_remote, id);
                ",
            )?;
            conn.pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION)?;
        }

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn.lock().map_err(|e| {
            CoreError::Storage(rusqlite::Error::ToSqlConversionFailure(Box::new(
                std::io::Error::other(e.to_string()),
            )))
        })
    }

    /// Inserts a new entry into history. If an entry with the same ID already exists, it is ignored.
    pub fn insert(&self, entry: &HistoryEntry) -> Result<()> {
        let conn = self.lock_conn()?;

        let kind_str = match entry.kind {
            ClipKind::Text => "text",
            ClipKind::Image => "image",
        };
        let is_remote_int = if entry.is_remote { 1 } else { 0 };

        conn.execute(
            "INSERT OR IGNORE INTO history (
                id, origin, kind, mime, hash, size, created_at, data, is_remote
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                entry.id,
                entry.origin,
                kind_str,
                entry.mime,
                entry.hash,
                entry.size as i64,
                entry.created_at,
                entry.data,
                is_remote_int,
            ],
        )?;

        Ok(())
    }

    /// Returns up to `limit` most recent history entries, newest first.
    pub fn recent(&self, limit: usize) -> Result<Vec<HistoryEntry>> {
        let conn = self.lock_conn()?;

        let mut stmt = conn.prepare(
            "SELECT id, origin, kind, mime, hash, size, created_at, data, is_remote
             FROM history
             ORDER BY created_at DESC, id DESC
             LIMIT ?1",
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            let kind_str: String = row.get(2)?;
            let kind = if kind_str == "image" {
                ClipKind::Image
            } else {
                ClipKind::Text
            };
            let size_i64: i64 = row.get(5)?;
            let is_remote_int: i64 = row.get(8)?;

            Ok(HistoryEntry {
                id: row.get(0)?,
                origin: row.get(1)?,
                kind,
                mime: row.get(3)?,
                hash: row.get(4)?,
                size: size_i64 as usize,
                created_at: row.get(6)?,
                data: row.get(7)?,
                is_remote: is_remote_int != 0,
            })
        })?;

        let mut entries = Vec::new();
        for r in rows {
            entries.push(r?);
        }
        Ok(entries)
    }

    /// Checks if an entry with the specified content hash exists in history.
    pub fn contains_hash(&self, hash: &[u8]) -> Result<bool> {
        let conn = self.lock_conn()?;

        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM history WHERE hash = ?1 LIMIT 1)",
            params![hash],
            |row| row.get(0),
        )?;

        Ok(exists)
    }

    /// Prunes entries created before `cutoff_ms` (Unix milliseconds).
    ///
    /// Returns the number of deleted records.
    pub fn prune_older_than(&self, cutoff_ms: i64) -> Result<usize> {
        let conn = self.lock_conn()?;

        let count = conn.execute("DELETE FROM history WHERE created_at < ?1", params![cutoff_ms])?;
        Ok(count)
    }

    /// Returns the greatest (most recent) envelope ULID received from a remote device.
    ///
    /// Used as the `since` query parameter when connecting to the relay server to fetch missed backlog.
    pub fn last_remote_envelope_id(&self) -> Result<Option<String>> {
        let conn = self.lock_conn()?;

        let mut stmt = conn.prepare(
            "SELECT id FROM history WHERE is_remote = 1 ORDER BY id DESC LIMIT 1",
        )?;

        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            Ok(Some(id))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_history_crud_and_duplicate_ignore() {
        let history = History::open_in_memory().unwrap();

        let entry1 = HistoryEntry {
            id: "01JC0000000000000000000001".to_string(),
            origin: "device-mac".to_string(),
            kind: ClipKind::Text,
            mime: "text/plain".to_string(),
            hash: vec![1, 2, 3],
            size: 5,
            created_at: 1000,
            data: b"hello".to_vec(),
            is_remote: false,
        };

        history.insert(&entry1).unwrap();

        // Duplicate insert should be silently ignored
        history.insert(&entry1).unwrap();

        let recent = history.recent(10).unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0], entry1);

        assert!(history.contains_hash(&[1, 2, 3]).unwrap());
        assert!(!history.contains_hash(&[4, 5, 6]).unwrap());
    }

    #[test]
    fn test_prune_and_last_remote_envelope_id() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("history.sqlite");
        let history = History::open(&db_path).unwrap();

        let local_entry = HistoryEntry {
            id: "01JC0000000000000000000010".to_string(),
            origin: "local-dev".to_string(),
            kind: ClipKind::Text,
            mime: "text/plain".to_string(),
            hash: vec![10],
            size: 4,
            created_at: 100,
            data: b"loc1".to_vec(),
            is_remote: false,
        };

        let remote_entry1 = HistoryEntry {
            id: "01JC0000000000000000000020".to_string(),
            origin: "remote-dev".to_string(),
            kind: ClipKind::Text,
            mime: "text/plain".to_string(),
            hash: vec![20],
            size: 4,
            created_at: 200,
            data: b"rem1".to_vec(),
            is_remote: true,
        };

        let remote_entry2 = HistoryEntry {
            id: "01JC0000000000000000000030".to_string(),
            origin: "remote-dev".to_string(),
            kind: ClipKind::Text,
            mime: "text/plain".to_string(),
            hash: vec![30],
            size: 4,
            created_at: 300,
            data: b"rem2".to_vec(),
            is_remote: true,
        };

        history.insert(&local_entry).unwrap();
        history.insert(&remote_entry1).unwrap();
        history.insert(&remote_entry2).unwrap();

        assert_eq!(
            history.last_remote_envelope_id().unwrap(),
            Some("01JC0000000000000000000030".to_string())
        );

        // Prune older than 250 ms -> local_entry (100) and remote_entry1 (200) should be pruned
        let pruned = history.prune_older_than(250).unwrap();
        assert_eq!(pruned, 2);

        let remaining = history.recent(10).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, remote_entry2.id);
    }
}
