CREATE TABLE imported_memory_domains (
  memory_id INTEGER PRIMARY KEY REFERENCES memories(id) ON DELETE CASCADE,
  domain TEXT NOT NULL CHECK(domain IN (
    'working','episode','semantic_user','relationship',
    'ai_self','procedural','commitment','reflection'
  ))
);
