//! Isolated persistence for encrypted activity context and proactive decisions.

use std::path::Path;
use std::time::Duration;

use pw_application::behavior::proactive::{FrequencyHistory, FrequencySnapshot, HistoryQuery};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use thiserror::Error;

const INITIAL_MIGRATION: &str = include_str!("../activity-migrations/0001_initial.sql");
const CURRENT_SCHEMA_VERSION: i64 = 1;
const ACTIVITY_SESSIONS_CREATE_SQL: &str = "
    CREATE TABLE activity_sessions (
        id INTEGER PRIMARY KEY,
        started_at INTEGER NOT NULL CHECK (started_at >= 0),
        ended_at INTEGER CHECK (ended_at IS NULL OR (ended_at >= 0 AND ended_at >= started_at)),
        duration_seconds INTEGER NOT NULL CHECK (duration_seconds >= 0),
        category TEXT NOT NULL,
        payload_version INTEGER NOT NULL,
        protected_context BLOB NOT NULL CHECK (length(protected_context) > 0)
    ) STRICT
";
const PROACTIVE_DECISIONS_CREATE_SQL: &str = "
    CREATE TABLE proactive_decisions (
        id INTEGER PRIMARY KEY,
        created_at INTEGER NOT NULL CHECK (created_at >= 0),
        candidate_kind TEXT NOT NULL,
        decision TEXT NOT NULL CHECK (decision IN ('speak', 'skip')),
        topic_hash BLOB NOT NULL UNIQUE CHECK (length(topic_hash) > 0)
    ) STRICT
";
const ACTIVITY_SESSION_COLUMNS: &[ColumnSpec] = &[
    ColumnSpec::new("id", "INTEGER", false, 1),
    ColumnSpec::new("started_at", "INTEGER", true, 0),
    ColumnSpec::new("ended_at", "INTEGER", false, 0),
    ColumnSpec::new("duration_seconds", "INTEGER", true, 0),
    ColumnSpec::new("category", "TEXT", true, 0),
    ColumnSpec::new("payload_version", "INTEGER", true, 0),
    ColumnSpec::new("protected_context", "BLOB", true, 0),
];
const PROACTIVE_DECISION_COLUMNS: &[ColumnSpec] = &[
    ColumnSpec::new("id", "INTEGER", false, 1),
    ColumnSpec::new("created_at", "INTEGER", true, 0),
    ColumnSpec::new("candidate_kind", "TEXT", true, 0),
    ColumnSpec::new("decision", "TEXT", true, 0),
    ColumnSpec::new("topic_hash", "BLOB", true, 0),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ColumnSpec {
    name: &'static str,
    declared_type: &'static str,
    not_null: bool,
    primary_key_position: i64,
}

impl ColumnSpec {
    const fn new(
        name: &'static str,
        declared_type: &'static str,
        not_null: bool,
        primary_key_position: i64,
    ) -> Self {
        Self {
            name,
            declared_type,
            not_null,
            primary_key_position,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ColumnMetadata {
    name: String,
    declared_type: String,
    not_null: bool,
    primary_key_position: i64,
    hidden: i64,
}

#[derive(Debug)]
struct IndexMetadata {
    name: String,
    unique: bool,
    origin: String,
    partial: bool,
}

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
    #[error("proactive topic hash is invalid")]
    InvalidTopicHash,
    #[error("proactive candidate kind is invalid")]
    InvalidCandidateKind,
    #[error("proactive frequency window is invalid")]
    InvalidFrequencyWindow,
    #[error("proactive frequency limit is invalid")]
    InvalidFrequencyLimit,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ActivityFrequencySnapshot {
    pub topic_exists: bool,
    pub latest_spoken_at: Option<i64>,
    pub spoken_last_hour: u64,
    pub spoken_last_day: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct FinalSpeakDecisionRequest<'a> {
    pub created_at: i64,
    pub candidate_kind: &'a str,
    pub topic_hash: &'a [u8],
    pub minimum_interval_seconds: i64,
    pub hour_since: i64,
    pub day_since: i64,
    pub max_per_hour: u64,
    pub max_per_day: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalSpeakDecisionOutcome {
    Inserted { decision_id: i64 },
    DuplicateTopic,
    RateLimited,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ActivityFrequencyHistoryError;

impl std::fmt::Debug for ActivityFrequencyHistoryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ActivityFrequencyHistoryError")
    }
}

impl std::fmt::Display for ActivityFrequencyHistoryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("activity frequency history unavailable")
    }
}

impl std::error::Error for ActivityFrequencyHistoryError {}

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
        if !table_columns_match(connection, "activity_sessions", ACTIVITY_SESSION_COLUMNS)?
            || !table_columns_match(
                connection,
                "proactive_decisions",
                PROACTIVE_DECISION_COLUMNS,
            )?
            || !table_create_sql_matches(
                connection,
                "activity_sessions",
                ACTIVITY_SESSIONS_CREATE_SQL,
            )?
            || !table_create_sql_matches(
                connection,
                "proactive_decisions",
                PROACTIVE_DECISIONS_CREATE_SQL,
            )?
            || !required_index_matches(
                connection,
                "activity_sessions",
                "activity_sessions_started_at_idx",
                false,
                "c",
                &["started_at"],
            )?
            || !required_index_matches(
                connection,
                "proactive_decisions",
                "proactive_decisions_created_at_idx",
                false,
                "c",
                &["created_at"],
            )?
            || !topic_hash_unique_index_matches(connection)?
        {
            return Err(ActivityStorageError::InvalidSchema);
        }
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
        let candidate_kind = validate_candidate_kind(candidate_kind)?;
        self.connection.execute(
            "INSERT INTO proactive_decisions(created_at,candidate_kind,decision,topic_hash) \
             VALUES (?1,?2,?3,?4)",
            params![created_at, candidate_kind, decision.as_str(), topic_hash],
        )?;
        Ok(self.connection.last_insert_rowid())
    }

    /// Loads all proactive rate-limit facts from one SQL statement.
    ///
    /// # Errors
    /// Returns an error for an invalid digest/window or failed `SQLite` query.
    pub fn frequency_snapshot(
        &self,
        topic_hash: &[u8],
        hour_since: i64,
        day_since: i64,
    ) -> Result<ActivityFrequencySnapshot, ActivityStorageError> {
        validate_topic_hash(topic_hash)?;
        validate_frequency_cutoffs(day_since, hour_since, None)?;
        query_frequency_snapshot(&self.connection, topic_hash, hour_since, day_since)
    }

    /// Atomically rechecks eligibility and records one final speak decision.
    ///
    /// # Errors
    /// Returns an error for invalid input or a failed `SQLite` transaction.
    pub fn record_final_speak(
        &mut self,
        request: FinalSpeakDecisionRequest<'_>,
    ) -> Result<FinalSpeakDecisionOutcome, ActivityStorageError> {
        validate_timestamp(request.created_at)?;
        validate_topic_hash(request.topic_hash)?;
        let candidate_kind = validate_candidate_kind(request.candidate_kind)?;
        validate_frequency_cutoffs(
            request.day_since,
            request.hour_since,
            Some(request.created_at),
        )?;
        if request.minimum_interval_seconds <= 0
            || request.max_per_hour == 0
            || request.max_per_day == 0
            || request.max_per_hour > i64::MAX.cast_unsigned()
            || request.max_per_day > i64::MAX.cast_unsigned()
        {
            return Err(ActivityStorageError::InvalidFrequencyLimit);
        }

        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let snapshot = query_frequency_snapshot(
            &transaction,
            request.topic_hash,
            request.hour_since,
            request.day_since,
        )?;
        if snapshot.topic_exists {
            return Ok(FinalSpeakDecisionOutcome::DuplicateTopic);
        }
        let interval_denied = snapshot.latest_spoken_at.is_some_and(|latest| {
            latest > request.created_at
                || request
                    .created_at
                    .checked_sub(latest)
                    .is_none_or(|elapsed| elapsed < request.minimum_interval_seconds)
        });
        if interval_denied
            || snapshot.spoken_last_hour >= request.max_per_hour
            || snapshot.spoken_last_day >= request.max_per_day
        {
            return Ok(FinalSpeakDecisionOutcome::RateLimited);
        }
        transaction.execute(
            "INSERT INTO proactive_decisions(created_at,candidate_kind,decision,topic_hash) \
             VALUES (?1,?2,'speak',?3)",
            params![request.created_at, candidate_kind, request.topic_hash],
        )?;
        let decision_id = transaction.last_insert_rowid();
        transaction.commit()?;
        Ok(FinalSpeakDecisionOutcome::Inserted { decision_id })
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

impl FrequencyHistory for ActivityDatabase {
    type Error = ActivityFrequencyHistoryError;

    fn snapshot(&self, query: HistoryQuery) -> Result<FrequencySnapshot, Self::Error> {
        self.frequency_snapshot(&query.topic_hash, query.hour_since, query.day_since)
            .map(|snapshot| FrequencySnapshot {
                topic_exists: snapshot.topic_exists,
                latest_spoken_at: snapshot.latest_spoken_at,
                spoken_last_hour: snapshot.spoken_last_hour,
                spoken_last_day: snapshot.spoken_last_day,
            })
            .map_err(|_| ActivityFrequencyHistoryError)
    }
}

fn query_frequency_snapshot(
    connection: &Connection,
    topic_hash: &[u8],
    hour_since: i64,
    day_since: i64,
) -> Result<ActivityFrequencySnapshot, ActivityStorageError> {
    let (topic_exists, latest_spoken_at, spoken_last_hour, spoken_last_day): (
        bool,
        Option<i64>,
        i64,
        i64,
    ) = connection.query_row(
        "SELECT \
         EXISTS(SELECT 1 FROM proactive_decisions WHERE topic_hash=?1), \
         MAX(CASE WHEN decision='speak' THEN created_at END), \
         COALESCE(SUM(CASE WHEN decision='speak' AND created_at>=?2 THEN 1 ELSE 0 END),0), \
         COALESCE(SUM(CASE WHEN decision='speak' AND created_at>=?3 THEN 1 ELSE 0 END),0) \
         FROM proactive_decisions",
        params![topic_hash, hour_since, day_since],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    Ok(ActivityFrequencySnapshot {
        topic_exists,
        latest_spoken_at,
        spoken_last_hour: u64::try_from(spoken_last_hour)
            .map_err(|_| ActivityStorageError::InvalidSchema)?,
        spoken_last_day: u64::try_from(spoken_last_day)
            .map_err(|_| ActivityStorageError::InvalidSchema)?,
    })
}

fn table_columns_match(
    connection: &Connection,
    table: &str,
    expected: &[ColumnSpec],
) -> Result<bool, ActivityStorageError> {
    let mut statement = connection.prepare(
        "SELECT name,type,\"notnull\",pk,hidden FROM pragma_table_xinfo(?1) ORDER BY cid",
    )?;
    let actual = statement
        .query_map([table], |row| {
            Ok(ColumnMetadata {
                name: row.get(0)?,
                declared_type: row.get(1)?,
                not_null: row.get::<_, i64>(2)? != 0,
                primary_key_position: row.get(3)?,
                hidden: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(actual.len() == expected.len()
        && actual.iter().zip(expected).all(|(actual, expected)| {
            actual.name == expected.name
                && actual.declared_type == expected.declared_type
                && actual.not_null == expected.not_null
                && actual.primary_key_position == expected.primary_key_position
                && actual.hidden == 0
        }))
}

fn table_create_sql_matches(
    connection: &Connection,
    table: &str,
    expected: &str,
) -> Result<bool, ActivityStorageError> {
    let actual: String = connection.query_row(
        "SELECT sql FROM sqlite_schema WHERE type='table' AND name=?1",
        [table],
        |row| row.get(0),
    )?;
    Ok(normalize_create_sql(&actual) == normalize_create_sql(expected))
}

fn normalize_create_sql(sql: &str) -> String {
    let mut normalized = String::with_capacity(sql.len());
    let mut inside_string = false;
    for character in sql.chars() {
        if character == '\'' {
            inside_string = !inside_string;
            normalized.push(character);
        } else if inside_string {
            normalized.push(character);
        } else if !character.is_ascii_whitespace() {
            normalized.push(character.to_ascii_lowercase());
        }
    }
    normalized
}

fn table_indexes(
    connection: &Connection,
    table: &str,
) -> Result<Vec<IndexMetadata>, ActivityStorageError> {
    let mut statement = connection
        .prepare("SELECT name,\"unique\",origin,partial FROM pragma_index_list(?1) ORDER BY seq")?;
    Ok(statement
        .query_map([table], |row| {
            Ok(IndexMetadata {
                name: row.get(0)?,
                unique: row.get::<_, i64>(1)? != 0,
                origin: row.get(2)?,
                partial: row.get::<_, i64>(3)? != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?)
}

fn index_key_columns(
    connection: &Connection,
    index: &str,
) -> Result<Vec<Option<String>>, ActivityStorageError> {
    let mut statement =
        connection.prepare("SELECT name FROM pragma_index_xinfo(?1) WHERE key=1 ORDER BY seqno")?;
    Ok(statement
        .query_map([index], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?)
}

fn index_key_columns_match(
    connection: &Connection,
    index: &str,
    expected: &[&str],
) -> Result<bool, ActivityStorageError> {
    let actual = index_key_columns(connection, index)?;
    Ok(actual.len() == expected.len()
        && actual
            .iter()
            .zip(expected)
            .all(|(actual, expected)| actual.as_deref() == Some(*expected)))
}

fn required_index_matches(
    connection: &Connection,
    table: &str,
    required_name: &str,
    unique: bool,
    origin: &str,
    columns: &[&str],
) -> Result<bool, ActivityStorageError> {
    let indexes = table_indexes(connection, table)?;
    let Some(index) = indexes.iter().find(|index| index.name == required_name) else {
        return Ok(false);
    };
    Ok(index.unique == unique
        && index.origin == origin
        && !index.partial
        && index_key_columns_match(connection, &index.name, columns)?)
}

fn topic_hash_unique_index_matches(connection: &Connection) -> Result<bool, ActivityStorageError> {
    for index in table_indexes(connection, "proactive_decisions")? {
        if index.unique
            && index.origin == "u"
            && !index.partial
            && index_key_columns_match(connection, &index.name, &["topic_hash"])?
        {
            return Ok(true);
        }
    }
    Ok(false)
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
    if topic_hash.len() != 32 {
        return Err(ActivityStorageError::InvalidTopicHash);
    }
    Ok(())
}

fn validate_candidate_kind(candidate_kind: &str) -> Result<&'static str, ActivityStorageError> {
    match candidate_kind {
        "return" => Ok("return"),
        "long_session" => Ok("long_session"),
        "category_change" => Ok("category_change"),
        _ => Err(ActivityStorageError::InvalidCandidateKind),
    }
}

fn validate_frequency_cutoffs(
    day_since: i64,
    hour_since: i64,
    created_at: Option<i64>,
) -> Result<(), ActivityStorageError> {
    if day_since < 0
        || day_since > hour_since
        || created_at.is_some_and(|created_at| hour_since > created_at)
    {
        return Err(ActivityStorageError::InvalidFrequencyWindow);
    }
    Ok(())
}
