-- v12: a promotion must be bound to the exact classified lease/run and its
-- action-to-candidate mapping.  Existing rows deliberately cannot satisfy a
-- v12 retry fingerprint, so they are never treated as a new retry result.
ALTER TABLE memory_promotions ADD COLUMN classification_run_id INTEGER NOT NULL DEFAULT 0;
ALTER TABLE memory_promotions ADD COLUMN change_set_fingerprint TEXT NOT NULL DEFAULT '';
CREATE UNIQUE INDEX memory_promotions_run_idx
  ON memory_promotions(classification_run_id, change_set_fingerprint)
  WHERE classification_run_id != 0;
