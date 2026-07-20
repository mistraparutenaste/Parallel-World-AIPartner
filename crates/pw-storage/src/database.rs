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
const MEMORY_UNIQUE_MIGRATION: &str = include_str!("../migrations/0006_memory_content_unique.sql");
const MEMORY_LIFECYCLE_MIGRATION: &str = include_str!("../migrations/0007_memory_lifecycle.sql");
const MESSAGES_ID_CURSOR_MIGRATION: &str =
    include_str!("../migrations/0008_messages_id_cursor.sql");
const MEMORY_TYPED_MIGRATION: &str = include_str!("../migrations/0009_typed_memory.sql");
const CURRENT_SCHEMA_VERSION: i64 = 9;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("SQLite operation failed")]
    Sqlite(#[from] rusqlite::Error),
    #[error("bundled SQLite {found} is older than required 3.51.3")]
    UnsupportedSqlite { found: String },
    #[error("database schema version {found} is newer than supported version {supported}")]
    FutureSchema { found: i64, supported: i64 },
}

pub struct Database {
    connection: Connection,
}

impl Database {
    /// Creates a consistent snapshot with `SQLite`'s Online Backup API.
    ///
    /// # Errors
    /// Returns an error when the destination cannot be opened or backup fails.
    pub fn backup_to(&self, destination: impl AsRef<Path>) -> Result<(), StorageError> {
        let mut destination = Connection::open(destination)?;
        let backup = rusqlite::backup::Backup::new(&self.connection, &mut destination)?;
        backup.run_to_completion(64, Duration::from_millis(10), None)?;
        Ok(())
    }
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
        if current > CURRENT_SCHEMA_VERSION {
            return Err(StorageError::FutureSchema {
                found: current,
                supported: CURRENT_SCHEMA_VERSION,
            });
        }
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
        let current: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if current < 6 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(MEMORY_UNIQUE_MIGRATION)?;
            transaction.pragma_update(None, "user_version", 6)?;
            transaction.commit()?;
        }
        let current: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if current < 7 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(MEMORY_LIFECYCLE_MIGRATION)?;
            transaction.pragma_update(None, "user_version", 7)?;
            transaction.commit()?;
        }
        let current: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if current < 8 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(MESSAGES_ID_CURSOR_MIGRATION)?;
            transaction.pragma_update(None, "user_version", 8)?;
            transaction.commit()?;
        }
        let current: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if current < 9 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(MEMORY_TYPED_MIGRATION)?;
            transaction.pragma_update(None, "user_version", 9)?;
            transaction.commit()?;
        }
        Ok(Self { connection })
    }

    #[must_use]
    pub const fn connection(&self) -> &Connection {
        &self.connection
    }

    pub const fn connection_mut(&mut self) -> &mut Connection {
        &mut self.connection
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        DETACHED_TURN_SEQUENCE_MIGRATION, Database, INITIAL_MIGRATION, MEMORY_FTS_MIGRATION,
        MEMORY_LIFECYCLE_MIGRATION, MEMORY_UNIQUE_MIGRATION, TURN_IDENTITY_MIGRATION,
        TURN_SEQUENCE_MIGRATION,
    };

    #[test]
    fn online_backup_produces_a_consistent_reopenable_snapshot() {
        let source =
            std::env::temp_dir().join(format!("pw-backup-source-{}.sqlite3", std::process::id()));
        let destination = std::env::temp_dir().join(format!(
            "pw-backup-destination-{}.sqlite3",
            std::process::id()
        ));
        let database = Database::open(&source).unwrap();
        database
            .connection()
            .execute(
                "INSERT INTO conversations(id,created_at,updated_at) VALUES('chat',1,1)",
                [],
            )
            .unwrap();
        database.backup_to(&destination).unwrap();
        let snapshot = Database::open(&destination).unwrap();
        let count: i64 = snapshot
            .connection()
            .query_row("SELECT COUNT(*) FROM conversations", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
        drop(snapshot);
        drop(database);
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_file(destination);
    }

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
            9
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
    fn rejects_database_from_a_future_schema_version() {
        let path = std::env::temp_dir().join(format!("pw-future-{}.sqlite3", std::process::id()));
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.pragma_update(None, "user_version", 10).unwrap();
        drop(connection);
        assert!(matches!(
            Database::open(&path),
            Err(super::StorageError::FutureSchema {
                found: 10,
                supported: 9
            })
        ));
        let _ = std::fs::remove_file(path);
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
            9
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
            9
        );
        drop(database);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn upgrades_v4_database_and_backfills_existing_memories_into_fts() {
        let path =
            std::env::temp_dir().join(format!("pw-v4-memory-{}.sqlite3", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let database = Database {
                connection: rusqlite::Connection::open(&path).unwrap(),
            };
            for migration in [
                INITIAL_MIGRATION,
                TURN_IDENTITY_MIGRATION,
                TURN_SEQUENCE_MIGRATION,
                DETACHED_TURN_SEQUENCE_MIGRATION,
            ] {
                database.connection().execute_batch(migration).unwrap();
            }
            database.connection().execute("INSERT INTO memories(content,created_at,updated_at) VALUES('既存の猫記憶',1,1)", []).unwrap();
            database
                .connection()
                .pragma_update(None, "user_version", 4)
                .unwrap();
        }
        let database = Database::open(&path).unwrap();
        let count: i64 = database
            .connection()
            .query_row(
                "SELECT count(*) FROM memories_fts WHERE memories_fts MATCH '\"既存の猫記憶\"'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn v6_upgrade_deduplicates_null_and_empty_sources_consistently_with_unique_index() {
        let path =
            std::env::temp_dir().join(format!("pw-v5-dedupe-{}.sqlite3", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let connection = rusqlite::Connection::open(&path).unwrap();
            for migration in [
                INITIAL_MIGRATION,
                TURN_IDENTITY_MIGRATION,
                TURN_SEQUENCE_MIGRATION,
                DETACHED_TURN_SEQUENCE_MIGRATION,
                MEMORY_FTS_MIGRATION,
            ] {
                connection.execute_batch(migration).unwrap();
            }
            connection.execute_batch("INSERT INTO conversations(id,created_at,updated_at) VALUES('',1,1); INSERT INTO memories(content,source_conversation_id,created_at,updated_at) VALUES('同じ記憶',NULL,1,1),('同じ記憶','',2,2);").unwrap();
            connection.pragma_update(None, "user_version", 5).unwrap();
        }
        let database = Database::open(&path).unwrap();
        let count: i64 = database
            .connection()
            .query_row(
                "SELECT count(*) FROM memories WHERE content='同じ記憶'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        assert!(database.connection().execute("INSERT INTO memories(content,source_conversation_id,created_at,updated_at) VALUES('同じ記憶','',3,3)", []).is_err());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn v7_upgrade_preserves_memory_and_adds_imported_grace_evidence() {
        let path =
            std::env::temp_dir().join(format!("pw-v7-upgrade-{}.sqlite3", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let connection = rusqlite::Connection::open(&path).unwrap();
            for migration in [
                INITIAL_MIGRATION,
                TURN_IDENTITY_MIGRATION,
                TURN_SEQUENCE_MIGRATION,
                DETACHED_TURN_SEQUENCE_MIGRATION,
                MEMORY_FTS_MIGRATION,
                MEMORY_UNIQUE_MIGRATION,
            ] {
                connection.execute_batch(migration).unwrap();
            }
            connection
                .execute(
                    "INSERT INTO memories(content,created_at,updated_at) VALUES('猫が好き',1,2)",
                    [],
                )
                .unwrap();
            connection.pragma_update(None, "user_version", 6).unwrap();
        }
        let database = Database::open(&path).unwrap();
        let row: (String, i64, i64) = database
            .connection()
            .query_row(
                "SELECT state,pinned,mention_count FROM memories WHERE content='猫が好き'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row, ("active".into(), 0, 1));
        let evidence: (String, f64) = database
            .connection()
            .query_row("SELECT kind,weight FROM memory_evidence", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(evidence, ("imported".into(), 1.0));
        drop(database);
        let reopened = Database::open(&path).unwrap();
        assert_eq!(
            reopened
                .connection()
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            9
        );
        drop(reopened);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn v9_upgrade_keeps_message_cursor_and_adds_typed_legacy_defaults() {
        let path =
            std::env::temp_dir().join(format!("pw-v8-id-cursor-{}.sqlite3", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let connection = rusqlite::Connection::open(&path).unwrap();
            for migration in [
                INITIAL_MIGRATION,
                TURN_IDENTITY_MIGRATION,
                TURN_SEQUENCE_MIGRATION,
                DETACHED_TURN_SEQUENCE_MIGRATION,
                MEMORY_FTS_MIGRATION,
                MEMORY_UNIQUE_MIGRATION,
                MEMORY_LIFECYCLE_MIGRATION,
            ] {
                connection.execute_batch(migration).unwrap();
            }
            connection.execute_batch("INSERT INTO memories(content,created_at,updated_at,state,pinned,mention_count,last_seen_at) VALUES('legacy typed projection',1,1,'active',0,1,1); PRAGMA user_version=7;").unwrap();
        }

        let database = Database::open(&path).unwrap();
        let index_exists: bool = database
            .connection()
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sqlite_master
                    WHERE type='index' AND name='messages_conversation_id_cursor'
                 )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let version: i64 = database
            .connection()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert!(index_exists);
        assert_eq!(version, 9);
        let typed: (i64, String, String, String, String, String, String, String) = database
            .connection()
            .query_row(
                "SELECT revision,subject_scope,epistemic_form,attribution,source_mode,speech_act,polarity,conditionality FROM memories WHERE content='legacy typed projection'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            typed,
            (
                1,
                "legacy_unknown".into(),
                "legacy_untyped".into(),
                "unknown".into(),
                "reported".into(),
                "unknown".into(),
                "unknown".into(),
                "unknown".into()
            )
        );
        drop(database);
        let reopened = Database::open(&path).unwrap();
        assert_eq!(
            reopened
                .connection()
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            9
        );
        drop(reopened);
        let _ = std::fs::remove_file(path);
    }
}
