DELETE FROM memories WHERE id NOT IN (SELECT MIN(id) FROM memories GROUP BY IFNULL(source_conversation_id, ''), content);
CREATE UNIQUE INDEX memories_source_content_unique ON memories(IFNULL(source_conversation_id, ''), content);
