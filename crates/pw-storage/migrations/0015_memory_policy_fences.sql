-- v15: policy metadata is immutable per classified proposal.  It lets a
-- retry make the same consent decision inside the promotion transaction.
ALTER TABLE memory_candidates ADD COLUMN memory_domain TEXT NOT NULL DEFAULT 'semantic_user'
  CHECK(memory_domain IN ('working','episode','semantic_user','relationship','ai_self','procedural','commitment','reflection'));
ALTER TABLE memory_candidates ADD COLUMN write_class TEXT NOT NULL DEFAULT 'normal_explicit'
  CHECK(write_class IN ('normal_explicit','inferred','personal','sensitive','secret','never_store'));
ALTER TABLE memory_candidates ADD COLUMN policy_state TEXT NOT NULL DEFAULT 'unreviewed'
  CHECK(policy_state IN ('unreviewed','auto_approved','pending_approval','rejected'));

-- Moving a conversation to temporary mode revokes every still-queued durable
-- derivation before another connection can claim or promote it.  Source rows
-- are removed (and cascade candidates/runs/promotions); transient state is
-- removed in the same SQLite statement that records the switch.
CREATE TRIGGER temporary_conversations_privacy_fence_insert
AFTER INSERT ON temporary_conversations WHEN NEW.temporary = 1
BEGIN
  DELETE FROM dialogue_states WHERE conversation_id = NEW.conversation_id;
  DELETE FROM commitments WHERE conversation_id = NEW.conversation_id;
  DELETE FROM memory_observations WHERE conversation_id = NEW.conversation_id;
END;
CREATE TRIGGER temporary_conversations_privacy_fence_update
AFTER UPDATE OF temporary ON temporary_conversations
WHEN NEW.temporary = 1 AND OLD.temporary != 1
BEGIN
  DELETE FROM dialogue_states WHERE conversation_id = NEW.conversation_id;
  DELETE FROM commitments WHERE conversation_id = NEW.conversation_id;
  DELETE FROM memory_observations WHERE conversation_id = NEW.conversation_id;
END;
