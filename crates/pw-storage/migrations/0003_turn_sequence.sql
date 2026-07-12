ALTER TABLE conversations ADD COLUMN next_turn_id INTEGER NOT NULL DEFAULT 1;
UPDATE conversations SET next_turn_id = MAX(
    next_turn_id,
    COALESCE((SELECT MAX(turn_id) + 1 FROM messages WHERE conversation_id = conversations.id), 1)
);
