# Task 5B implementation report

## Primary-source research

- SQLite `BEGIN IMMEDIATE`: <https://www.sqlite.org/lang_transaction.html> and <https://www.sqlite.org/isolation.html>. SQLite documents that an immediate transaction starts the write transaction up front, serializes competing writers, and avoids a later `SQLITE_BUSY_SNAPSHOT` upgrade failure. The final speak path therefore opens `TransactionBehavior::Immediate` before its snapshot/recheck.
- rusqlite transaction mapping: <https://docs.rs/rusqlite/0.40.1/rusqlite/enum.TransactionBehavior.html> and <https://docs.rs/rusqlite/0.40.1/src/rusqlite/transaction.rs.html>. `TransactionBehavior::Immediate` maps to `BEGIN IMMEDIATE`; the worker-owned mutable connection is used for the transaction.
- SQLite isolation: <https://www.sqlite.org/isolation.html>. Separate connections see complete committed transactions, and a transaction retains a consistent snapshot. The frequency facts are returned by one SQL statement; the final decision rechecks those facts inside the immediate write transaction.

## RED evidence

`cargo test -p pw-storage activity` failed before production changes with unresolved imports/methods for:

- `FinalSpeakDecisionRequest` / `FinalSpeakDecisionOutcome`;
- `ActivityDatabase::frequency_snapshot` / `record_final_speak`;
- `FrequencyHistory for ActivityDatabase`;
- `ProactiveAssistantHistory` / `ProactiveAssistantMessage`;
- `SqliteConversationHistory::append_proactive_assistant`.

The failure was the expected missing-feature failure, not a fixture or syntax failure.

## GREEN implementation

- Added a one-statement activity frequency snapshot and opaque `FrequencyHistory` adapter.
- Kept activity schema-v1 SQL/validator unchanged; exact 32-byte digests and the three candidate strings are enforced at the Rust boundary.
- Added `BEGIN IMMEDIATE` atomic final-speak recheck/insert with typed inserted/duplicate/rate-limited outcomes and duplicate precedence.
- Added a separate proactive assistant history port and one-transaction assistant-only append.
- Normal reservations and proactive appends share an integer-only, checked detached-sequence allocator; no SQLite arithmetic can promote an overflow to `REAL`.
- Proactive content is bounded at 65,536 UTF-8 bytes; all proactive adapter errors have constant Display/Debug and no source chain.

## Verification evidence

- `cargo test -p pw-storage --test activity`: 27 passed, including inclusive boundaries, skip exclusion, malformed hashes/candidates, exact interval, future timestamp, hour/day limits, duplicate precedence, two-connection race, opaque adapter failure, and pre-Task5 v1 reopen.
- `cargo test -p pw-storage --test proactive_assistant`: 8 passed, including assistant-only rows, monotonic/reopen interoperability, 64 KiB byte boundary, invalid inputs, message/final-update rollback, opaque trigger failures, and overflow persistence.
- `cargo test -p pw-application --test history`: 2 passed.
- `cargo test -p pw-storage`: 64 passed across unit/integration targets; 0 failed.
- `cargo test -p pw-application`: 75 passed across unit/integration targets; 0 failed.
- `cargo clippy -p pw-application -p pw-storage --all-targets -- -D warnings`: passed after correcting two documentation-markdown warnings.

The brief's named acceptance commands are rerun after this report is written so the final commit records fresh output.
