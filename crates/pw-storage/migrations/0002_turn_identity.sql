WITH duplicate_turns AS (
    SELECT id,
           MAX(turn_id) OVER (PARTITION BY conversation_id) AS max_turn,
           DENSE_RANK() OVER (PARTITION BY conversation_id ORDER BY turn_id, id) AS ordinal
    FROM messages
    WHERE turn_id IS NOT NULL
), conflicting AS (
    SELECT id, max_turn + ordinal AS replacement
    FROM duplicate_turns d
    WHERE EXISTS (
        SELECT 1 FROM messages other
        WHERE other.conversation_id = (SELECT conversation_id FROM messages WHERE id = d.id)
          AND other.turn_id = (SELECT turn_id FROM messages WHERE id = d.id)
          AND other.role = (SELECT role FROM messages WHERE id = d.id)
          AND other.id < d.id
    )
)
UPDATE messages SET turn_id = (SELECT replacement FROM conflicting WHERE conflicting.id = messages.id)
WHERE id IN (SELECT id FROM conflicting);

CREATE UNIQUE INDEX messages_conversation_turn_role
    ON messages(conversation_id, turn_id, role) WHERE turn_id IS NOT NULL;
