-- v13: separated long-lived state controls.  These tables contain bounded
-- metadata only; source text remains in the observation ledger.
CREATE TABLE memory_domain_controls (
  domain TEXT PRIMARY KEY CHECK(domain IN ('working','episode','semantic_user','relationship','ai_self','procedural','commitment','reflection')),
  consent TEXT NOT NULL CHECK(consent IN ('allowed','pending_approval','never_store')),
  retention_seconds INTEGER CHECK(retention_seconds IS NULL OR retention_seconds > 0),
  revision INTEGER NOT NULL DEFAULT 0 CHECK(revision >= 0),
  updated_at INTEGER NOT NULL
);

INSERT INTO memory_domain_controls(domain,consent,retention_seconds,revision,updated_at) VALUES
  ('working','allowed',NULL,0,0),
  ('episode','pending_approval',NULL,0,0),
  ('semantic_user','allowed',NULL,0,0),
  ('relationship','pending_approval',NULL,0,0),
  ('ai_self','pending_approval',NULL,0,0),
  ('procedural','pending_approval',NULL,0,0),
  ('commitment','pending_approval',NULL,0,0),
  ('reflection','pending_approval',NULL,0,0);

CREATE TABLE memory_versions (
  memory_id INTEGER NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
  revision INTEGER NOT NULL CHECK(revision > 0),
  content_hash TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY(memory_id, revision)
);

CREATE TABLE memory_links (
  from_memory_id INTEGER NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
  to_memory_id INTEGER NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
  relation TEXT NOT NULL CHECK(relation IN ('supports','supersedes','contradicts','derived_from')),
  created_at INTEGER NOT NULL,
  PRIMARY KEY(from_memory_id,to_memory_id,relation),
  CHECK(from_memory_id != to_memory_id)
);

-- Kept after a memory row disappears.  The generation fences any late writer
-- which only knows an earlier version of the memory.
CREATE TABLE memory_tombstones (
  memory_id INTEGER NOT NULL,
  generation INTEGER NOT NULL CHECK(generation > 0),
  deleted_at INTEGER NOT NULL,
  final_support_removed INTEGER NOT NULL CHECK(final_support_removed IN (0,1)),
  pinned INTEGER NOT NULL CHECK(pinned IN (0,1)),
  PRIMARY KEY(memory_id, generation)
);
CREATE INDEX memory_tombstones_latest_idx ON memory_tombstones(memory_id,generation DESC);

CREATE TABLE commitments (
  id INTEGER PRIMARY KEY,
  conversation_id TEXT NOT NULL,
  content TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('open','completed','cancelled','expired')),
  due_at INTEGER,
  next_check_at INTEGER,
  expires_at INTEGER,
  revision INTEGER NOT NULL DEFAULT 0 CHECK(revision >= 0),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX commitments_expiry_idx ON commitments(status,expires_at,next_check_at,id);

CREATE TABLE temporary_conversations (
  conversation_id TEXT PRIMARY KEY,
  temporary INTEGER NOT NULL CHECK(temporary IN (0,1)),
  revision INTEGER NOT NULL DEFAULT 0 CHECK(revision >= 0),
  updated_at INTEGER NOT NULL
);
