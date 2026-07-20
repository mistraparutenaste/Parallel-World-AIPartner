-- v11: durable queue recovery and privacy fences.  We retain only a
-- content-free observation tombstone after history removal so provenance can
-- remain auditable without retaining the user's utterance or candidates.
ALTER TABLE memory_observations ADD COLUMN retry_after_at INTEGER;
ALTER TABLE memory_observations ADD COLUMN deleted_at INTEGER;
ALTER TABLE memory_candidates ADD COLUMN normalization_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE memory_provenance ADD COLUMN tombstoned_at INTEGER;
CREATE INDEX memory_observations_retry_idx
  ON memory_observations(processing_state, retry_after_at, observed_at, id);
