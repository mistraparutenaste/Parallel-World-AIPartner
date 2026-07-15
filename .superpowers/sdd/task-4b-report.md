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

## Review-fix addendum (2026-07-15)

### Additional dependency research

- reqwest 0.12.28 documents that its default client may retry safe low-level
  protocol NACKs. `reqwest::retry::never()` is the documented policy that disables
  this default behavior. The evaluator builder now fixes this policy explicitly;
  it remains one request with no retry. Sources:
  https://docs.rs/reqwest/0.12.28/reqwest/retry/fn.never.html and the locally
  resolved `reqwest-0.12.28/src/retry.rs`.
- The locally resolved reqwest 0.12.28 manifest defines
  `rustls-tls-native-roots` as the Rustls/ring backend plus native root certificate
  loading. The workspace reqwest feature list now includes that exact feature.
  `cargo tree -p pw-llm -e features` confirms `hyper-rustls`,
  `rustls-native-certs 0.8.4`, Rustls `ring`, `std`, and `tls12` in the effective
  graph. Source: https://docs.rs/crate/reqwest/0.12.28/features
- TLS test support uses `rcgen 0.14.8` and `rustls 0.23.41` as dev dependencies.
  The test creates a localhost SAN certificate, runs a bounded Rustls server, and
  supplies that certificate only through a private `cfg(test)` root-injection
  constructor. Production does not call any invalid-certificate or invalid-hostname
  bypass. Sources: https://docs.rs/rcgen/0.14.8/rcgen/ and
  https://docs.rs/rustls/0.23.41/rustls/struct.ConfigBuilder.html

### Additional RED/GREEN evidence

1. RED: `cargo test -p pw-application proactive_session_ids`
   - The old engine treated session id 2 -> 1 as a new session and emitted Return.
   - GREEN: decreasing/reused ids now reset continuity; only strictly greater
     SQLite row ids establish a new session.
2. RED: `cargo test -p pw-llm evaluator_https_uses_certificate_validation`
   - Failed to compile because `EVALUATOR_MAX_RETRIES` and the private test-root
     constructor did not exist (`E0432`, `E0599`).
   - GREEN: a real localhost TLS handshake and strict Speak response pass with the
     injected trusted root while normal certificate validation remains active.
3. `evaluator_failed_response_is_sent_exactly_once` verifies one 503 response is
   received exactly once. The production builder also fixes
   `.retry(reqwest::retry::never())`, preventing default protocol-NACK retries.

### Review-fix acceptance

```text
cargo test -p pw-application proactive
PASS: 1 unit + 10 integration proactive tests

cargo test -p pw-llm evaluator
PASS: 5 unit + 13 integration evaluator tests

cargo fmt --all --check
PASS

cargo clippy -p pw-application -p pw-llm --all-targets -- -D warnings
PASS

git diff --check
PASS (Windows LF-to-CRLF warnings only)

cargo test -p pw-application
PASS: 73 tests across package targets

cargo test -p pw-llm
PASS: 24 automated tests; 1 real-server test ignored
```
