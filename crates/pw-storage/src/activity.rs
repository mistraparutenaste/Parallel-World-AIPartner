//! Isolated persistence for encrypted activity context and proactive decisions.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;

const INITIAL_MIGRATION: &str = include_str!("../activity-migrations/0001_initial.sql");
const CURRENT_SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Error)]
pub enum ActivityStorageError {
    #[error("SQLite activity operation failed")]
    Sqlite(#[from] rusqlite::Error),
    #[error("activity database schema version {found} is newer than supported version {supported}")]
    FutureSchema { found: i64, supported: i64 },
    #[error("activity database schema is invalid")]
    InvalidSchema,
    #[error("activity timestamp must be nonnegative")]
    InvalidTimestamp,
    #[error("activity duration must be nonnegative")]
    InvalidDuration,
    #[error("activity session end precedes its start")]
    EndedBeforeStarted,
    #[error("protected activity context must not be empty")]
    EmptyProtectedContext,
    #[error("proactive topic hash must not be empty")]
    EmptyTopicHash,
    #[error("activity page size must be greater than zero")]
    InvalidPageSize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewActivitySession {
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub duration_seconds: i64,
    pub category: String,
    pub payload_version: u16,
    pub protected_context: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredActivitySession {
    pub id: i64,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub duration_seconds: i64,
    pub category: String,
    pub payload_version: u16,
    pub protected_context: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivitySessionPage {
    pub sessions: Vec<StoredActivitySession>,
    pub next_before_id: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProactiveDecision {
    Speak,
    Skip,
}

impl ProactiveDecision {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Speak => "speak",
            Self::Skip => "skip",
        }
    }
}

/// Dedicated activity database. It never applies conversation or memory migrations.
pub struct ActivityDatabase {
    connection: Connection,
}

impl ActivityDatabase {
    /// Opens or creates a file-backed activity database.
    ///
    /// # Errors
    /// Returns an error for open/configuration failures, unsupported future
    /// versions, corrupt schemas, or failed migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ActivityStorageError> {
        let connection = Connection::open(path)?;
        Self::configure(connection, true)
    }

    /// Opens a migrated in-memory activity database.
    ///
    /// # Errors
    /// Returns an error when configuration or migration fails.
    pub fn open_in_memory() -> Result<Self, ActivityStorageError> {
        let connection = Connection::open_in_memory()?;
        Self::configure(connection, false)
    }

    fn configure(
        mut connection: Connection,
        enable_wal: bool,
    ) -> Result<Self, ActivityStorageError> {
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.busy_timeout(Duration::from_secs(5))?;

        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > CURRENT_SCHEMA_VERSION {
            return Err(ActivityStorageError::FutureSchema {
                found: version,
                supported: CURRENT_SCHEMA_VERSION,
            });
        }

        if enable_wal {
            connection.pragma_update(None, "journal_mode", "WAL")?;
        }
        if version == 0 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(INITIAL_MIGRATION)?;
            transaction.pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION)?;
            transaction.commit()?;
        }

        Self::validate_schema(&connection)?;
        Ok(Self { connection })
    }

    fn validate_schema(connection: &Connection) -> Result<(), ActivityStorageError> {
        let integrity: String =
            connection.pragma_query_value(None, "quick_check", |row| row.get(0))?;
        if integrity != "ok" {
            return Err(ActivityStorageError::InvalidSchema);
        }
        let strict_table_count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM pragma_table_list \
             WHERE schema='main' AND type='table' AND strict=1 \
             AND name IN ('activity_sessions','proactive_decisions')",
            [],
            |row| row.get(0),
        )?;
        if strict_table_count != 2 {
            return Err(ActivityStorageError::InvalidSchema);
        }
        connection
            .prepare(
                "SELECT id,started_at,ended_at,duration_seconds,category,payload_version,protected_context \
                 FROM activity_sessions LIMIT 0",
            )
            .map_err(|_| ActivityStorageError::InvalidSchema)?;
        connection
            .prepare(
                "SELECT id,created_at,candidate_kind,decision,topic_hash \
                 FROM proactive_decisions LIMIT 0",
            )
            .map_err(|_| ActivityStorageError::InvalidSchema)?;
        Ok(())
    }

    #[must_use]
    pub const fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Inserts one session containing opaque protected context only.
    ///
    /// # Errors
    /// Returns an error for invalid values or a failed `SQLite` write.
    pub fn insert_session(
        &mut self,
        session: &NewActivitySession,
    ) -> Result<i64, ActivityStorageError> {
        validate_session(session)?;
        self.connection.execute(
            "INSERT INTO activity_sessions \
             (started_at,ended_at,duration_seconds,category,payload_version,protected_context) \
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                session.started_at,
                session.ended_at,
                session.duration_seconds,
                session.category,
                session.payload_version,
                session.protected_context,
            ],
        )?;
        Ok(self.connection.last_insert_rowid())
    }

    /// Completes or updates the timing of an existing session.
    ///
    /// # Errors
    /// Returns an error for invalid values, an end before the stored start, or
    /// a failed `SQLite` operation.
    pub fn update_session(
        &mut self,
        id: i64,
        ended_at: Option<i64>,
        duration_seconds: i64,
    ) -> Result<bool, ActivityStorageError> {
        validate_optional_timestamp(ended_at)?;
        validate_duration(duration_seconds)?;
        let started_at = self
            .connection
            .query_row(
                "SELECT started_at FROM activity_sessions WHERE id=?1",
                [id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let Some(started_at) = started_at else {
            return Ok(false);
        };
        if ended_at.is_some_and(|ended| ended < started_at) {
            return Err(ActivityStorageError::EndedBeforeStarted);
        }
        Ok(self.connection.execute(
            "UPDATE activity_sessions SET ended_at=?1,duration_seconds=?2 WHERE id=?3",
            params![ended_at, duration_seconds, id],
        )? > 0)
    }

    /// Returns newest sessions, optionally restricted to ids below a cursor.
    ///
    /// # Errors
    /// Returns an error for a zero page size or failed `SQLite` query.
    pub fn page_sessions(
        &self,
        limit: u32,
        before_id: Option<i64>,
    ) -> Result<ActivitySessionPage, ActivityStorageError> {
        if limit == 0 {
            return Err(ActivityStorageError::InvalidPageSize);
        }
        let fetch_limit = i64::from(limit) + 1;
        let mut statement = self.connection.prepare(
            "SELECT id,started_at,ended_at,duration_seconds,category,payload_version,protected_context \
             FROM activity_sessions WHERE (?1 IS NULL OR id < ?1) \
             ORDER BY id DESC LIMIT ?2",
        )?;
        let mut sessions = statement
            .query_map(params![before_id, fetch_limit], stored_session_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        let has_more = sessions.len() > limit as usize;
        if has_more {
            sessions.pop();
        }
        let next_before_id = if has_more {
            sessions.last().map(|session| session.id)
        } else {
            None
        };
        Ok(ActivitySessionPage {
            sessions,
            next_before_id,
        })
    }

    /// Deletes sessions selected by their database ids.
    ///
    /// # Errors
    /// Returns an error when a `SQLite` operation fails.
    pub fn delete_selected_sessions(&mut self, ids: &[i64]) -> Result<usize, ActivityStorageError> {
        let transaction = self.connection.transaction()?;
        let mut deleted = 0;
        for id in ids {
            deleted += transaction.execute("DELETE FROM activity_sessions WHERE id=?1", [id])?;
        }
        transaction.commit()?;
        Ok(deleted)
    }

    /// Deletes sessions whose start is strictly earlier than `cutoff`.
    ///
    /// # Errors
    /// Returns an error for a negative cutoff or failed `SQLite` deletion.
    pub fn delete_sessions_before(&mut self, cutoff: i64) -> Result<usize, ActivityStorageError> {
        validate_timestamp(cutoff)?;
        Ok(self.connection.execute(
            "DELETE FROM activity_sessions WHERE started_at < ?1",
            [cutoff],
        )?)
    }

    /// Deletes every stored activity session.
    ///
    /// # Errors
    /// Returns an error when the `SQLite` deletion fails.
    pub fn delete_all_sessions(&mut self) -> Result<usize, ActivityStorageError> {
        Ok(self
            .connection
            .execute("DELETE FROM activity_sessions", [])?)
    }

    /// Records a speak/skip decision without free-form reason text.
    ///
    /// # Errors
    /// Returns an error for invalid values, duplicate topic hashes, or a failed
    /// `SQLite` write.
    pub fn insert_proactive_decision(
        &mut self,
        created_at: i64,
        candidate_kind: &str,
        decision: ProactiveDecision,
        topic_hash: &[u8],
    ) -> Result<i64, ActivityStorageError> {
        validate_timestamp(created_at)?;
        validate_topic_hash(topic_hash)?;
        self.connection.execute(
            "INSERT INTO proactive_decisions(created_at,candidate_kind,decision,topic_hash) \
             VALUES (?1,?2,?3,?4)",
            params![created_at, candidate_kind, decision.as_str(), topic_hash],
        )?;
        Ok(self.connection.last_insert_rowid())
    }

    /// Checks whether a topic hash was already considered.
    ///
    /// # Errors
    /// Returns an error for an empty hash or failed `SQLite` query.
    pub fn has_topic_hash(&self, topic_hash: &[u8]) -> Result<bool, ActivityStorageError> {
        validate_topic_hash(topic_hash)?;
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM proactive_decisions WHERE topic_hash=?1)",
            [topic_hash],
            |row| row.get(0),
        )?)
    }

    /// Counts spoken decisions at or after the inclusive timestamp boundary.
    ///
    /// # Errors
    /// Returns an error for a negative timestamp or failed `SQLite` query.
    pub fn count_decisions_since(&self, since: i64) -> Result<u64, ActivityStorageError> {
        validate_timestamp(since)?;
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM proactive_decisions \
             WHERE decision='speak' AND created_at >= ?1",
            [since],
            |row| row.get(0),
        )?;
        Ok(count.cast_unsigned())
    }

    /// Returns the timestamp of the most recent spoken decision.
    ///
    /// # Errors
    /// Returns an error when the `SQLite` query fails.
    pub fn latest_spoken_decision_at(&self) -> Result<Option<i64>, ActivityStorageError> {
        Ok(self.connection.query_row(
            "SELECT MAX(created_at) FROM proactive_decisions WHERE decision='speak'",
            [],
            |row| row.get(0),
        )?)
    }
}

fn stored_session_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredActivitySession> {
    Ok(StoredActivitySession {
        id: row.get(0)?,
        started_at: row.get(1)?,
        ended_at: row.get(2)?,
        duration_seconds: row.get(3)?,
        category: row.get(4)?,
        payload_version: row.get(5)?,
        protected_context: row.get(6)?,
    })
}

fn validate_session(session: &NewActivitySession) -> Result<(), ActivityStorageError> {
    validate_timestamp(session.started_at)?;
    validate_optional_timestamp(session.ended_at)?;
    validate_duration(session.duration_seconds)?;
    if session
        .ended_at
        .is_some_and(|ended| ended < session.started_at)
    {
        return Err(ActivityStorageError::EndedBeforeStarted);
    }
    if session.protected_context.is_empty() {
        return Err(ActivityStorageError::EmptyProtectedContext);
    }
    Ok(())
}

fn validate_timestamp(timestamp: i64) -> Result<(), ActivityStorageError> {
    if timestamp < 0 {
        return Err(ActivityStorageError::InvalidTimestamp);
    }
    Ok(())
}

fn validate_optional_timestamp(timestamp: Option<i64>) -> Result<(), ActivityStorageError> {
    if let Some(timestamp) = timestamp {
        validate_timestamp(timestamp)?;
    }
    Ok(())
}

fn validate_duration(duration: i64) -> Result<(), ActivityStorageError> {
    if duration < 0 {
        return Err(ActivityStorageError::InvalidDuration);
    }
    Ok(())
}

fn validate_topic_hash(topic_hash: &[u8]) -> Result<(), ActivityStorageError> {
    if topic_hash.is_empty() {
        return Err(ActivityStorageError::EmptyTopicHash);
    }
    Ok(())
}
