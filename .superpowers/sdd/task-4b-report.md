# Task 4B implementation report

## Scope

Implemented only the deterministic proactive candidate/rate/interaction-gate core in
`pw-application` and the bounded `Speak`/`Skip` evaluator in `pw-llm`. No desktop
orchestration, natural reply generation, persona/chat persistence, UI/TTS, worker,
or Tauri IPC was added.

## Dependency and protocol research

- RustCrypto `sha2`: the primary crate documentation describes the `Digest`/
  `Sha256` incremental API used here. The workspace lock already resolved
  `sha2 0.10.9`; the workspace dependency was therefore added as `sha2 = "0.10"`
  to reuse the compatible audited lock resolution instead of introducing a second
  major line. Source: https://docs.rs/sha2/latest/sha2/
- reqwest: the workspace dependency resolves to `reqwest 0.12.28`. Its blocking
  `ClientBuilder::timeout` covers connect/read/write operations, and redirects can
  be disabled with `redirect(Policy::none())`; the blocking response implements
  `Read`, enabling an explicit `take(limit + 1)` bound. Source:
  https://docs.rs/reqwest/0.12/reqwest/blocking/struct.ClientBuilder.html
- OpenAI structured outputs: the official Chat Completions contract recommends
  `response_format.type = "json_schema"` with a named `json_schema`, `strict: true`,
  and a JSON Schema. The request follows that shape with
  `additionalProperties: false`. Source:
  https://platform.openai.com/docs/api-reference/chat/create
- llama.cpp: current server and grammar documentation advertise schema-constrained
  JSON for the OpenAI-compatible chat-completions endpoint, including
  `response_format` JSON-schema forms. Compatibility is treated as an endpoint
  assumption: there is no capability probe or fallback retry, and incompatible
  output becomes `Skip`. Sources:
  https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md and
  https://github.com/ggml-org/llama.cpp/blob/master/grammars/README.md

## TDD evidence

### RED

1. `cargo test -p pw-application proactive`
   - Failed at compile time with `E0433: could not find behavior in pw_application`.
   - This was the expected failure before the candidate/rate/gate module existed.
2. `cargo test -p pw-llm evaluator`
   - Failed at compile time with `E0432` for the four missing evaluator API exports.
   - This was the expected failure before the evaluator implementation existed.
3. The first golden-vector run failed with the placeholder expected digest. The
   canonical byte sequence was independently recomputed with .NET SHA-256 and the
   test was fixed to the confirmed vector
   `b18ab75f5a5684aa3c1256d05d2ae2eada181c153d9624f58b69d1a2bdff6744`.

### GREEN

- `cargo test -p pw-application proactive`
  - Passed: 1 poison/fail-closed unit test and 9 proactive integration tests.
- `cargo test -p pw-llm evaluator`
  - Passed: 4 evaluator unit tests and 12 evaluator integration tests.
- Candidate coverage includes exact return/long/category boundaries, bounce,
  cross-session pending category, trigger priority/delayed lower trigger, stable
  SHA-256 identities, invalid/nonmonotonic reset, one-call frequency snapshots,
  epoch-near cutoffs, future/error fail-closed behavior, and interaction epochs.
- Evaluator coverage includes exact inner JSON, strict known outer wrapper, partial
  and invalid zero-call configuration, selected-pair-only validation, remote policy,
  header/body timeouts, non-2xx/3xx, transport failure, redirects disabled, normal
  and chunked oversized bodies, realistic OpenAI metadata, and typed request shape.

## Limits and compatibility assumptions

- Production overall HTTP timeout: exactly 8 seconds.
- Response wrapper: explicitly read at most 16 KiB + 1 byte; more than 16 KiB skips.
- Evaluator model: trimmed and limited to 1..=128 Unicode characters.
- Typed duration context: 0..=604800 seconds; longer input is invalid and skips at
  the adapter boundary.
- Request: one non-streaming call, temperature 0, max 16 output tokens, no retry,
  no redirects, and no raw application/title field.
- The selected OpenAI-compatible endpoint must understand the OpenAI nested
  `json_schema` response-format shape and return a compatible single-choice wrapper.
  Known OpenAI metadata and llama.cpp `timings` are accepted; unexpected outer or
  inner fields fail closed.
- Restart intentionally loses previous/pending continuity (safe under-trigger).
  Long-session events can be rediscovered and are deduplicated later by the stable
  topic hash through the one-call history port.

## Final acceptance

```text
cargo test -p pw-application proactive
PASS: 1 unit + 9 integration proactive tests

cargo test -p pw-llm evaluator
PASS: 4 unit + 12 integration evaluator tests

cargo fmt --all --check
PASS

cargo clippy -p pw-application -p pw-llm --all-targets -- -D warnings
PASS

git diff --check
PASS (Git emitted only the existing Windows LF-to-CRLF conversion warnings)

cargo test -p pw-application
PASS: all package unit/integration/doc tests (72 passed total across targets)

cargo test -p pw-llm
PASS: all automated package unit/integration/doc tests (22 passed, 1 real-server test ignored)
```

## Concerns / deferred work

- The Task 2 SQLite repository intentionally remains outside this crate boundary;
  a later desktop adapter must implement the single history snapshot port and must
  re-run the same eligibility function immediately before decision commit.
- The interaction gate is implemented and tested but is not connected to ChatService
  in this task, as required by scope.
