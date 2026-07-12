CREATE TABLE conversation_turn_sequences (
    conversation_id TEXT PRIMARY KEY NOT NULL,
    next_turn_id INTEGER NOT NULL CHECK(next_turn_id > 0)
);
INSERT INTO conversation_turn_sequences (conversation_id, next_turn_id)
SELECT id, next_turn_id FROM conversations;
