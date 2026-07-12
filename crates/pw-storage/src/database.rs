use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;
use thiserror::Error;

const INITIAL_MIGRATION: &str = include_str!("../migrations/0001_initial.sql");
const TURN_IDENTITY_MIGRATION: &str = include_str!("../migrations/0002_turn_identity.sql");
const TURN_SEQUENCE_MIGRATION: &str = include_str!("../migrations/0003_turn_sequence.sql");
const DETACHED_TURN_SEQUENCE_MIGRATION: &str =
    include_str!("../migrations/0004_detached_turn_sequence.sql");
const MEMORY_FTS_MIGRATION: &str = include_str!("../migrations/0005_memory_fts.sql");

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("SQLite operation failed")]
    Sqlite(#[from] rusqlite::Error),
    #[error("bundled SQLite {found} is older than required 3.51.3")]
    UnsupportedSqlite { found: String },
}

pub struct Database {
    connection: Connection,
}

impl Database {
    /// Opens or creates the application database at `path`.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot open/configure the database, a
    /// migration fails, or the bundled `SQLite` is too old for safe WAL use.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let connection = Connection::open(path)?;
        Self::configure(connection, true)
    }

    /// Opens a migrated in-memory database for tests and ephemeral work.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot configure or migrate the database,
    /// or when the bundled `SQLite` version is unsupported.
    pub fn open_in_memory() -> Result<Self, StorageError> {
        let connection = Connection::open_in_memory()?;
        Self::configure(connection, false)
    }

    fn configure(mut connection: Connection, enable_wal: bool) -> Result<Self, StorageError> {
        let version = rusqlite::version().to_owned();
        if rusqlite::version_number() < 3_051_003 {
            return Err(StorageError::UnsupportedSqlite { found: version });
        }
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        if enable_wal {
            connection.pragma_update(None, "journal_mode", "WAL")?;
        }
        let current: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if current == 0 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(INITIAL_MIGRATION)?;
            transaction.pragma_update(None, "user_version", 1)?;
            transaction.commit()?;
        }
        let current: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if current < 2 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(TURN_IDENTITY_MIGRATION)?;
            transaction.pragma_update(None, "user_version", 2)?;
            transaction.commit()?;
        }
        let current: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if current < 3 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(TURN_SEQUENCE_MIGRATION)?;
            transaction.pragma_update(None, "user_version", 3)?;
            transaction.commit()?;
        }
        let current: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if current < 4 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(DETACHED_TURN_SEQUENCE_MIGRATION)?;
            transaction.pragma_update(None, "user_version", 4)?;
            transaction.commit()?;
        }
        let current: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if current < 5 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(MEMORY_FTS_MIGRATION)?;
            transaction.pragma_update(None, "user_version", 5)?;
            transaction.commit()?;
        }
        Ok(Self { connection })
    }

    #[must_use]
    pub const fn connection(&self) -> &Connection {
        &self.connection
    }

    pub(crate) const fn connection_mut(&mut self) -> &mut Connection {
        &mut self.connection
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{Database, INITIAL_MIGRATION};

    #[test]
    fn in_memory_database_applies_connection_pragmas_and_schema() {
        let database = Database::open_in_memory().expect("database opens");
        let connection = database.connection();

        assert_eq!(
            connection
                .pragma_query_value(None, "foreign_keys", |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .pragma_query_value(None, "busy_timeout", |row| row.get::<_, i64>(0))
                .unwrap(),
            5_000
        );
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            5
        );

        for table in [
            "conversations",
            "messages",
            "conversation_summaries",
            "memories",
        ] {
            let exists: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(exists, "missing table {table}");
        }
    }

    #[test]
    fn file_database_uses_wal_and_survives_reopen() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("parallel-world-storage-{nonce}.sqlite3"));

        {
            let database = Database::open(&path).expect("database opens");
            let mode: String = database
                .connection()
                .pragma_query_value(None, "journal_mode", |row| row.get(0))
                .unwrap();
            assert_eq!(mode, "wal");
        }
        let reopened = Database::open(&path).expect("database reopens");
        assert_eq!(
            reopened
                .connection()
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            5
        );

        drop(reopened);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn upgrades_v1_database_with_duplicate_turn_roles_deterministically() {
        let path =
            std::env::temp_dir().join(format!("pw-v1-upgrade-{}.sqlite3", std::process::id()));
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.execute_batch(INITIAL_MIGRATION).unwrap();
        connection.execute_batch(
            "INSERT INTO conversations VALUES ('chat',1,1);
             INSERT INTO messages (conversation_id,turn_id,role,content,created_at) VALUES
             ('chat',1,'user','first',1),('chat',1,'user','duplicate',2),('chat',1,'assistant','reply',3);
             PRAGMA user_version=1;"
        ).unwrap();
        drop(connection);
        let database = Database::open(&path).unwrap();
        let turns: Vec<i64> = {
            let mut statement = database
                .connection()
                .prepare("SELECT turn_id FROM messages ORDER BY id")
                .unwrap();
            statement
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert_eq!(turns[0], 1);
        assert_ne!(turns[1], 1);
        assert_eq!(
            database
                .connection()
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            5
        );
        drop(database);
        let _ = std::fs::remove_file(path);
    }
}
