CREATE TABLE activity_sessions (
    id INTEGER PRIMARY KEY,
    started_at INTEGER NOT NULL CHECK (started_at >= 0),
    ended_at INTEGER CHECK (ended_at IS NULL OR (ended_at >= 0 AND ended_at >= started_at)),
    duration_seconds INTEGER NOT NULL CHECK (duration_seconds >= 0),
    category TEXT NOT NULL,
    payload_version INTEGER NOT NULL,
    protected_context BLOB NOT NULL CHECK (length(protected_context) > 0)
) STRICT;

CREATE INDEX activity_sessions_started_at_idx
    ON activity_sessions(started_at);

CREATE TABLE proactive_decisions (
    id INTEGER PRIMARY KEY,
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    candidate_kind TEXT NOT NULL,
    decision TEXT NOT NULL CHECK (decision IN ('speak', 'skip')),
    topic_hash BLOB NOT NULL UNIQUE CHECK (length(topic_hash) > 0)
) STRICT;

CREATE INDEX proactive_decisions_created_at_idx
    ON proactive_decisions(created_at);
