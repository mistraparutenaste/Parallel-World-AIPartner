CREATE UNIQUE INDEX messages_conversation_turn_role
    ON messages(conversation_id, turn_id, role) WHERE turn_id IS NOT NULL;
