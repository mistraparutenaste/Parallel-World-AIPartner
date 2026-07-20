CREATE TABLE memory_observations (
  id INTEGER PRIMARY KEY,
  conversation_id TEXT NOT NULL,
  turn_id INTEGER NOT NULL CHECK(turn_id >= 0),
  user_text TEXT NOT NULL,
  input_hash TEXT NOT NULL,
  observed_at INTEGER NOT NULL,
  response_outcome TEXT NOT NULL DEFAULT 'pending' CHECK(response_outcome IN ('pending','completed','cancelled','llm_failed','history_persist_failed','interrupted')),
  processing_state TEXT NOT NULL DEFAULT 'pending' CHECK(processing_state IN ('pending','processing','completed','deferred')),
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK(attempt_count >= 0),
  last_error TEXT,
  lease_owner TEXT,
  lease_expires_at INTEGER,
  attempt_token TEXT,
  deletion_generation INTEGER NOT NULL DEFAULT 0 CHECK(deletion_generation >= 0),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE(conversation_id, turn_id),
  CHECK((processing_state = 'processing') = (lease_owner IS NOT NULL AND lease_expires_at IS NOT NULL AND attempt_token IS NOT NULL))
);
CREATE INDEX memory_observations_claim_idx ON memory_observations(processing_state, lease_expires_at, observed_at, id);

CREATE TABLE memory_classification_runs (
  id INTEGER PRIMARY KEY,
  observation_id INTEGER NOT NULL REFERENCES memory_observations(id) ON DELETE CASCADE,
  classifier_version TEXT NOT NULL,
  schema_version INTEGER NOT NULL CHECK(schema_version > 0),
  input_hash TEXT NOT NULL,
  lease_attempt_token TEXT NOT NULL,
  transport_outcome TEXT NOT NULL CHECK(transport_outcome IN ('pending','completed','failed','rejected')),
  candidate_count INTEGER NOT NULL DEFAULT 0 CHECK(candidate_count >= 0),
  error_reason TEXT,
  created_at INTEGER NOT NULL,
  completed_at INTEGER,
  UNIQUE(observation_id, classifier_version, schema_version, input_hash, lease_attempt_token)
);

CREATE TABLE memory_candidates (
  id INTEGER PRIMARY KEY,
  observation_id INTEGER NOT NULL REFERENCES memory_observations(id) ON DELETE CASCADE,
  classification_run_id INTEGER NOT NULL REFERENCES memory_classification_runs(id) ON DELETE CASCADE,
  candidate_ordinal INTEGER NOT NULL CHECK(candidate_ordinal >= 0),
  content TEXT NOT NULL,
  subject_scope TEXT NOT NULL CHECK(subject_scope IN ('user_self','external_world','other_person','fictional_subject','legacy_unknown')),
  epistemic_form TEXT NOT NULL CHECK(epistemic_form IN ('fact_claim','belief','impression','prediction_or_hunch','metaphor','emotion','legacy_untyped')),
  attribution TEXT NOT NULL CHECK(attribution IN ('user','assistant','named_third_party','external_source','unknown')),
  speech_act TEXT NOT NULL CHECK(speech_act IN ('asserted','questioned','unknown')),
  source_mode TEXT NOT NULL CHECK(source_mode IN ('direct','reported','quoted')),
  polarity TEXT NOT NULL CHECK(polarity IN ('affirmed','negated','unknown')),
  conditionality TEXT NOT NULL CHECK(conditionality IN ('actual','hypothetical','unknown')),
  fictionality TEXT NOT NULL CHECK(fictionality IN ('real_world','fictional','unknown')),
  verification_status TEXT NOT NULL CHECK(verification_status IN ('not_applicable','user_reported','unverified_external_claim','externally_corroborated','externally_contradicted','disputed','unknown')),
  temporal_scope TEXT NOT NULL CHECK(temporal_scope IN ('stable','current','past','future','unknown')),
  target_memory_id INTEGER REFERENCES memories(id) ON DELETE SET NULL,
  expected_target_revision INTEGER CHECK(expected_target_revision > 0),
  proposed_operation TEXT NOT NULL CHECK(proposed_operation IN ('add','reinforce','supersede')),
  proposed_relation TEXT NOT NULL CHECK(proposed_relation IN ('originated','reasserted','corrected','changed_stance','contradicted')),
  source_start INTEGER NOT NULL CHECK(source_start >= 0),
  source_end INTEGER NOT NULL CHECK(source_end > source_start),
  candidate_state TEXT NOT NULL DEFAULT 'pending' CHECK(candidate_state IN ('pending','promoted','rejected')),
  rejection_reason TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE(observation_id, classification_run_id, candidate_ordinal),
  CHECK((target_memory_id IS NULL) = (expected_target_revision IS NULL))
);

CREATE TABLE memory_promotions (
  request_key TEXT PRIMARY KEY,
  observation_id INTEGER NOT NULL REFERENCES memory_observations(id) ON DELETE CASCADE,
  classifier_version TEXT NOT NULL,
  schema_version INTEGER NOT NULL CHECK(schema_version > 0),
  input_hash TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('committed')),
  result_memory_ids TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  committed_at INTEGER NOT NULL
);

CREATE TABLE memory_provenance (
  memory_id INTEGER NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
  observation_id INTEGER NOT NULL REFERENCES memory_observations(id) ON DELETE CASCADE,
  candidate_id INTEGER NOT NULL REFERENCES memory_candidates(id) ON DELETE CASCADE,
  relation TEXT NOT NULL CHECK(relation IN ('originated','reasserted','corrected','changed_stance','contradicted')),
  created_at INTEGER NOT NULL,
  PRIMARY KEY(memory_id, observation_id, relation)
);
