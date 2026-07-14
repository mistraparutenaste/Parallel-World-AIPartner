ALTER TABLE memories ADD COLUMN state TEXT NOT NULL DEFAULT 'active'
  CHECK(state IN ('active','dormant','superseded'));
ALTER TABLE memories ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0,1));
ALTER TABLE memories ADD COLUMN mention_count INTEGER NOT NULL DEFAULT 1 CHECK(mention_count > 0);
ALTER TABLE memories ADD COLUMN last_seen_at INTEGER NOT NULL DEFAULT 0;
ALTER TABLE memories ADD COLUMN state_changed_at INTEGER;
ALTER TABLE memories ADD COLUMN superseded_by INTEGER REFERENCES memories(id) ON DELETE SET NULL;
UPDATE memories SET last_seen_at = updated_at;

CREATE TABLE memory_evidence (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  memory_id INTEGER NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK(kind IN ('user_mention','recalled','pinned','imported')),
  occurred_at INTEGER NOT NULL,
  source_conversation_id TEXT,
  source_turn_id INTEGER,
  weight REAL NOT NULL CHECK(weight >= 0.0)
);
CREATE UNIQUE INDEX memory_evidence_turn_unique
  ON memory_evidence(memory_id,kind,source_conversation_id,source_turn_id)
  WHERE source_conversation_id IS NOT NULL AND source_turn_id IS NOT NULL;
CREATE INDEX memory_evidence_memory_time ON memory_evidence(memory_id,occurred_at);
CREATE INDEX memories_lifecycle_state ON memories(state,pinned,state_changed_at);
INSERT INTO memory_evidence(memory_id,kind,occurred_at,weight)
  SELECT id,'imported',CAST(strftime('%s','now') AS INTEGER),1.0 FROM memories;
