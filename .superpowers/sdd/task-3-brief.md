# Task 3: Add domain metadata, version/link/tombstone controls, commitments, dialogue state, and async state writers.

## Context

Task 2 provides durable observation/candidate/promotion storage. Add the separated
state required by the approved five-state design without slowing ordinary turns.

## Scope

- Add schema v13 for memory domains, per-domain consent/retention controls, memory
  version/link/tombstone records, commitments, and temporary-conversation settings.
- Add schema v14 for dialogue state (mood/relationship/reaction/reflection state can be
  represented here when it is needed by the existing domain model) with version/CAS and
  bounded expiry.
- Add application/storage contracts and asynchronous state writers. User-facing turn
  path must enqueue bounded work and continue if a writer fails.
- Enforce policy: normal explicit facts auto-approved; inferred/personal/sensitive pending;
  secrets/never-store rejected; temporary chat never writes durable memory/relationship.
- Add deterministic deletion/tombstone generation checks so delete cannot be resurrected
  by a late observation/promotion/state write.

## Constraints

- Preserve schema 1–10 and current lifecycle/FTS/redaction behavior.
- SQLite writes must use transaction/CAS semantics; no partial database encryption.
- No UI or planner implementation in this task; expose contracts for Task 5/6.
- No external API calls.

## Verification/deliverables

- Migration/reopen, domain-control, version-link, tombstone race, commitment expiry,
  dialogue-state CAS, temporary-chat, and fail-open writer tests.
- `cargo fmt --all -- --check`, focused Rust suites, desktop lib.
- Commit and `.superpowers/sdd/task-3-report.md`.
