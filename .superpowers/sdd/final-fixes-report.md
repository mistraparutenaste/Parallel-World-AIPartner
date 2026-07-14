# Memory lifecycle final fixes

Date: 2026-07-14
Base: `e5ad218`
Scope: final review Important 1-5; backend Rust and existing tests only.

## Research and design constraints

- SQLite FTS5 primary documentation was checked before implementation. FTS input is quoted phrase-by-phrase, BM25 lower-is-better ordering is converted to higher-is-better relevance, and final ranking happens after a bounded lexical pool is loaded.
- Rust standard-library `Arc`/atomic guidance and the existing `LlmClient` cancellation contract were checked. The enrichment classifier now receives a worker-owned shared cancellation flag.
- No migration, schema-version change, or new dependency was required.

## Important 1: consolidation candidate discovery

RED:

- Added real SQLite test `contradiction_statement_discovers_the_previous_memory_safely`.
- `cargo test -p pw-storage contradiction_statement_discovers -- --nocapture` failed because `私は犬が好き` did not retrieve stored `私は猫が好き`.

GREEN:

- Replaced the whole-statement FTS phrase with up to 16 safely quoted, evenly sampled trigram phrases joined by `OR`.
- FTS/LIKE result pools remain bounded. Empty input and `limit == 0` return no rows; under-three-character input uses the escaped LIKE fallback.
- The real SQLite test covers the contradiction pair plus empty, short, FTS-syntax-shaped, and Unicode inputs.
- Added production enrichment test `enrichment_supersedes_a_lexically_related_contradiction`; the old row becomes `superseded` and only the replacement remains active.

## Important 2: pin-aware Supersede

RED:

- `cargo test -p pw-application supersede_ -- --nocapture` failed because a valid Supersede containing `忘れないで` still produced `pin_replacement: false`.

GREEN:

- Valid Supersede now sets `pin_replacement` from `has_explicit_pin_intent(statement)`.
- Direct tests cover ordinary Supersede, explicit `忘れないで`, unknown IDs, and ungrounded replacement content.

## Important 3: maintenance reachability

RED:

- Added real SQLite/fake-time test with 100 strong rows followed by one weak row.
- `cargo test -p pw-storage maintenance_cursor_reaches -- --nocapture` failed because the second bounded pass reevaluated the first 100 rows.
- Added production worker test with `remaining: true`; it failed because the next pass remained scheduled 60 seconds later.

GREEN:

- `SqliteMemoryStore` uses process-local keyset cursors for active and expired phases. Cursor state advances only after transaction commit.
- `MaintenanceReport::remaining` tells the context worker whether another bounded pass is required.
- Production follows remaining work after 100 ms; completed/error cycles retain the 24-hour production interval. Each pass remains one bounded transaction.
- A database reopen resets the cursor to the first page, which may repeat work but cannot permanently starve later rows because remaining pages are followed within that worker lifetime.

## Important 4: rerank oversampling

RED:

- Added real FTS test `prompt_rerank_oversamples_beyond_the_final_limit`.
- It failed because SQL truncated to five BM25 rows before strength reranking.

GREEN:

- Search loads `final_limit * 4`, capped at 100, normalizes lexical relevance and strength across that pool, applies 70/30 ranking, then truncates to the requested final count.
- The real FTS test proves a stronger row below the first five lexical rows enters the final five.

## Important 5: cancellable enrichment shutdown

RED:

- Added a real `OpenAiCompatClient` test with a 30-second configured timeout and a server that withholds response headers.
- Cancellation at 50 ms still waited about two seconds and returned a transport error, failing the under-500-ms cancellation assertion.
- This RED did not use a dedicated short timeout.

GREEN:

- `LlmMemoryClassifier` owns a reusable shared `Arc<AtomicBool>` and no longer creates a fresh always-false flag per classification.
- Production enrichment creates a worker-owned cancel flag; `Worker::shutdown` sets it before joining threads. Reset and settings-driven restart use this same shutdown path.
- Blocking HTTP/SSE work runs in an internal helper and delivers deltas through a capacity-16 channel. The caller polls cancellation every 20 ms and returns immediately when cancelled.
- Active HTTP helper threads are globally capped at 16. A helper that outlives a cancelled caller remains bounded by the original request timeout and cannot deliver classifier output or mutate memory after its receiver is dropped.
- Tests cover cancellation while waiting for headers and Worker shutdown while a classifier request is in flight.

## Focused verification

- `cargo fmt --all --check`: exit 0.
- `cargo test -p pw-application memory -- --nocapture`: exit 0, 19 passed.
- `cargo test -p pw-storage memory -- --nocapture`: exit 0, 15 passed.
- `cargo test -p pw-llm --test contract -- --nocapture`: exit 0, 6 passed.
- `cargo test -p parallel-world-desktop enrichment -- --nocapture`: exit 0, 9 passed.
- `cargo test -p parallel-world-desktop memory_context -- --nocapture`: exit 0, 3 passed.
- `cargo test -p parallel-world-desktop maintenance -- --nocapture`: exit 0, 4 passed.
- `cargo test -p parallel-world-desktop shutdown -- --nocapture`: exit 0, 6 passed.

Desktop commands used `SHERPA_ONNX_LIB_DIR=E:\app\parallel-world\target\sherpa-onnx-prebuilt\sherpa-onnx-v1.13.4-win-x64-static-MT-Release-lib\lib`.

## Minor cleanup

- Replaced the fixed 20 ms maintenance-starvation sleep with a one-second deadline/yield loop to reduce CI timing flakiness.

## Final verification

- `cargo test -p pw-application`: exit 0; 43 unit tests plus integration/doc tests passed.
- `cargo test -p pw-storage`: exit 0; 29 tests plus doc tests passed.
- `cargo test -p parallel-world-desktop`: exit 0; 110 unit, 10 capability, 1 updater fixture, and doc tests passed.
- `cargo fmt --all --check`: exit 0.
- `cargo clippy --workspace --all-targets -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all runnable workspace tests passed. Seven environment-dependent tests remained ignored as before (audio hardware, real LLM, downloaded STT models, and real TTS engine).
