-- v16: temporary mode is a durable privacy boundary.  v15 removed the
-- observation graph, but a promoted memory does not reverse-cascade from its
-- provenance.  Tombstone and remove an unpinned memory when every remaining
-- live source belongs to the conversation being made temporary.
DROP TRIGGER temporary_conversations_privacy_fence_insert;
DROP TRIGGER temporary_conversations_privacy_fence_update;

CREATE TRIGGER temporary_conversations_privacy_fence_insert
AFTER INSERT ON temporary_conversations WHEN NEW.temporary = 1
BEGIN
  DELETE FROM dialogue_states WHERE conversation_id = NEW.conversation_id;
  DELETE FROM commitments WHERE conversation_id = NEW.conversation_id;
  INSERT INTO memory_tombstones(memory_id,generation,deleted_at,final_support_removed,pinned)
  SELECT p.memory_id,
         COALESCE((SELECT MAX(t.generation) FROM memory_tombstones t WHERE t.memory_id=p.memory_id),0)+1,
         NEW.updated_at,1,0
    FROM memory_provenance p
    JOIN memory_observations o ON o.id=p.observation_id
    JOIN memories m ON m.id=p.memory_id
   WHERE o.conversation_id=NEW.conversation_id
     AND o.deleted_at IS NULL
     AND m.pinned=0
   GROUP BY p.memory_id
  HAVING NOT EXISTS (
    SELECT 1
      FROM memory_provenance live_p
      JOIN memory_observations live_o ON live_o.id=live_p.observation_id
     WHERE live_p.memory_id=p.memory_id
       AND live_o.deleted_at IS NULL
       AND live_o.conversation_id<>NEW.conversation_id
  );
  DELETE FROM memories
   WHERE id IN (
    SELECT p.memory_id
      FROM memory_provenance p
      JOIN memory_observations o ON o.id=p.observation_id
      JOIN memories m ON m.id=p.memory_id
     WHERE o.conversation_id=NEW.conversation_id
       AND o.deleted_at IS NULL
       AND m.pinned=0
     GROUP BY p.memory_id
    HAVING NOT EXISTS (
      SELECT 1
        FROM memory_provenance live_p
        JOIN memory_observations live_o ON live_o.id=live_p.observation_id
       WHERE live_p.memory_id=p.memory_id
         AND live_o.deleted_at IS NULL
         AND live_o.conversation_id<>NEW.conversation_id
    )
   );
  DELETE FROM memory_observations WHERE conversation_id = NEW.conversation_id;
END;

CREATE TRIGGER temporary_conversations_privacy_fence_update
AFTER UPDATE OF temporary ON temporary_conversations
WHEN NEW.temporary = 1 AND OLD.temporary != 1
BEGIN
  DELETE FROM dialogue_states WHERE conversation_id = NEW.conversation_id;
  DELETE FROM commitments WHERE conversation_id = NEW.conversation_id;
  INSERT INTO memory_tombstones(memory_id,generation,deleted_at,final_support_removed,pinned)
  SELECT p.memory_id,
         COALESCE((SELECT MAX(t.generation) FROM memory_tombstones t WHERE t.memory_id=p.memory_id),0)+1,
         NEW.updated_at,1,0
    FROM memory_provenance p
    JOIN memory_observations o ON o.id=p.observation_id
    JOIN memories m ON m.id=p.memory_id
   WHERE o.conversation_id=NEW.conversation_id
     AND o.deleted_at IS NULL
     AND m.pinned=0
   GROUP BY p.memory_id
  HAVING NOT EXISTS (
    SELECT 1
      FROM memory_provenance live_p
      JOIN memory_observations live_o ON live_o.id=live_p.observation_id
     WHERE live_p.memory_id=p.memory_id
       AND live_o.deleted_at IS NULL
       AND live_o.conversation_id<>NEW.conversation_id
  );
  DELETE FROM memories
   WHERE id IN (
    SELECT p.memory_id
      FROM memory_provenance p
      JOIN memory_observations o ON o.id=p.observation_id
      JOIN memories m ON m.id=p.memory_id
     WHERE o.conversation_id=NEW.conversation_id
       AND o.deleted_at IS NULL
       AND m.pinned=0
     GROUP BY p.memory_id
    HAVING NOT EXISTS (
      SELECT 1
        FROM memory_provenance live_p
        JOIN memory_observations live_o ON live_o.id=live_p.observation_id
       WHERE live_p.memory_id=p.memory_id
         AND live_o.deleted_at IS NULL
         AND live_o.conversation_id<>NEW.conversation_id
    )
   );
  DELETE FROM memory_observations WHERE conversation_id = NEW.conversation_id;
END;
