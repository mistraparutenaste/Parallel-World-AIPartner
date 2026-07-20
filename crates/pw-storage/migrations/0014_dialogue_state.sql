-- v14: bounded, reconstructable interaction state.  It is deliberately
-- separate from historical messages and expires without affecting memories.
CREATE TABLE dialogue_states (
  conversation_id TEXT PRIMARY KEY,
  mood TEXT,
  relationship_summary TEXT,
  relationship_score INTEGER CHECK(relationship_score BETWEEN -100 AND 100),
  reaction TEXT,
  reflection_cursor TEXT,
  reflection_state TEXT,
  expires_at INTEGER NOT NULL,
  revision INTEGER NOT NULL DEFAULT 0 CHECK(revision >= 0),
  deletion_generation INTEGER NOT NULL DEFAULT 0 CHECK(deletion_generation >= 0),
  updated_at INTEGER NOT NULL,
  CHECK(length(COALESCE(mood,'')) <= 1024),
  CHECK(length(COALESCE(relationship_summary,'')) <= 1024),
  CHECK(length(COALESCE(reaction,'')) <= 1024),
  CHECK(length(COALESCE(reflection_cursor,'')) <= 1024),
  CHECK(length(COALESCE(reflection_state,'')) <= 1024)
);
CREATE INDEX dialogue_states_expiry_idx ON dialogue_states(expires_at,conversation_id);
