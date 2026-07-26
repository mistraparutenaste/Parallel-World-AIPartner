CREATE TABLE self_reviews (
  conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
  content TEXT NOT NULL,
  generated_at INTEGER NOT NULL,
  source_message_id INTEGER REFERENCES messages(id) ON DELETE SET NULL
);
