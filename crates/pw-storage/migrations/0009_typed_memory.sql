ALTER TABLE memories ADD COLUMN revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0);
ALTER TABLE memories ADD COLUMN subject_scope TEXT NOT NULL DEFAULT 'legacy_unknown'
  CHECK(subject_scope IN ('user_self','external_world','other_person','fictional_subject','legacy_unknown'));
ALTER TABLE memories ADD COLUMN epistemic_form TEXT NOT NULL DEFAULT 'legacy_untyped'
  CHECK(epistemic_form IN ('fact_claim','belief','impression','prediction_or_hunch','metaphor','emotion','legacy_untyped'));
ALTER TABLE memories ADD COLUMN attribution TEXT NOT NULL DEFAULT 'unknown'
  CHECK(attribution IN ('user','assistant','named_third_party','external_source','unknown'));
ALTER TABLE memories ADD COLUMN speech_act TEXT NOT NULL DEFAULT 'asserted'
  CHECK(speech_act IN ('asserted','questioned'));
ALTER TABLE memories ADD COLUMN source_mode TEXT NOT NULL DEFAULT 'reported'
  CHECK(source_mode IN ('direct','reported','quoted'));
ALTER TABLE memories ADD COLUMN polarity TEXT NOT NULL DEFAULT 'affirmed'
  CHECK(polarity IN ('affirmed','negated'));
ALTER TABLE memories ADD COLUMN conditionality TEXT NOT NULL DEFAULT 'actual'
  CHECK(conditionality IN ('actual','hypothetical'));
ALTER TABLE memories ADD COLUMN fictionality TEXT NOT NULL DEFAULT 'unknown'
  CHECK(fictionality IN ('real_world','fictional','unknown'));
ALTER TABLE memories ADD COLUMN verification_status TEXT NOT NULL DEFAULT 'unknown'
  CHECK(verification_status IN ('not_applicable','user_reported','unverified_external_claim','externally_corroborated','externally_contradicted','disputed','unknown'));
ALTER TABLE memories ADD COLUMN temporal_scope TEXT NOT NULL DEFAULT 'unknown'
  CHECK(temporal_scope IN ('stable','current','past','future','unknown'));
ALTER TABLE memories ADD COLUMN named_entity TEXT;
ALTER TABLE memories ADD COLUMN target_memory_id INTEGER REFERENCES memories(id) ON DELETE SET NULL;
ALTER TABLE memories ADD COLUMN target_revision INTEGER CHECK(target_revision IS NULL OR target_revision > 0);
ALTER TABLE memories ADD COLUMN stance_strength REAL CHECK(stance_strength IS NULL OR (stance_strength >= 0.0 AND stance_strength <= 1.0));
ALTER TABLE memories ADD COLUMN emotion_intensity REAL CHECK(emotion_intensity IS NULL OR (emotion_intensity >= 0.0 AND emotion_intensity <= 1.0));
ALTER TABLE memories ADD COLUMN validity_start_at INTEGER;
ALTER TABLE memories ADD COLUMN validity_end_at INTEGER;
