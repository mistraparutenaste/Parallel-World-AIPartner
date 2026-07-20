# Task 6 report: relationship, mood, reflection, and proactive integration

## Implemented

- Added `DialogueSignals` and deterministic bounded signal/explicit-commitment
  derivation. Completed turns carry no transcript in companion state.
- Added `CompanionStateWorker` with a bounded fail-open queue. Domain consent,
  temporary mode, CAS, expiry, and privacy fences are rechecked in the worker.
- Added bounded planned-turn state retrieval (`SqlitePlannedStateContext`) and
  a `StateAwareRetriever`; simple turns continue to bypass all retrieval.
- Connected completed desktop turns to the async state queue without changing
  the single streamed LLM/TTS path.
- Added proactive grant helper that composes `InteractionGate`, frequency
  history, master/profile/snooze, temporary, policy-error, and active-turn
  cancellation checks.
- Hardened explicit commitment matching to phrase allow-lists and added an
  atomic idle-epoch claim to prevent duplicate proactive grants.
- Prefix-only Japanese cues (`必ず`/`次は`) now require an action phrase;
  invalid negative snooze timestamps fail closed.
- Planned retrieval now uses one `IMMEDIATE` SQLite snapshot for temporary
  status, consent controls, dialogue state, and commitments.
- Async control writes preserve payload revisions for CAS.

## Verification

- `cargo fmt --all`
- `cargo test -p pw-application proactive --no-default-features` (all pass)
- `cargo test -p pw-application companion --no-default-features` (all pass)
- `cargo test -p pw-application routing --no-default-features` (all pass on rerun; one 1ms budget test was scheduler-flaky once)
- `cargo test -p pw-storage state_worker --no-default-features` (all pass)
- `cargo test -p pw-storage planned_snapshot --no-default-features` (pass)
- `cargo test -p parallel-world-desktop --lib chat::service --no-default-features` with cached Sherpa assets (64 pass)
- `cargo check -p parallel-world-desktop --lib` (pass)

Known unrelated frontend/live2d baseline failures remain outside this task.
