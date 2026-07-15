# Task 3 report: Windows activity collector core

## Outcome

Implemented the backend-only opt-in Windows foreground activity collector core.

- Added a narrow `ForegroundContextSource` platform port and Windows implementation with foreground HWND/PID revalidation, self-process exclusion before title access, query-limited process handles with RAII cleanup, basename-only app identifiers, bounded 512-unit titles, wrapping idle arithmetic, and optional fullscreen detection.
- Added checked behavior-settings loading and bounded, case-insensitive literal activity exclusions.
- Added a five-second stoppable collector worker with consent-v1/collection gates, continuity breaks on gaps and failures, separate DPAPI envelopes for app/title, basic app categories, encrypted session compression, retention cadence, stable health, disconnected-channel shutdown, and join-outside-lock cleanup.
- Kept Task 3 core-only: no IPC, UI, tray, shortcut, or startup wiring was added.
- Restored unrelated generated Tauri schema/permission line-ending churn after verification.

## TDD and review notes

Work resumed from the existing in-progress RED/GREEN state without discarding its uncommitted implementation. The inherited collector suite was first re-run and was GREEN (13 tests). Final strict linting then exposed two implementation issues and several test/style issues:

- Win32 title-buffer capacity used an unchecked `usize` to `i32` cast. It now uses a checked conversion.
- Collector test ports returned unit errors and lacked required public error documentation. They now use opaque stable error types with documented failure contracts.
- The data-only `ModeProfileDto` intentionally contains four independent transport flags, so it has a narrowly scoped `struct_excessive_bools` allowance.
- Existing Task 1 test setup was mechanically adjusted to satisfy the repository's strict clippy profile without changing behavior.

The collector tests were re-run after the fixes and remained GREEN (13/13).

## Acceptance commands and outputs

### `cargo test -p pw-platform activity`

Exit code: `0`

```text
running 4 tests
test activity::foreground_helper_tests::activity_fullscreen_is_unknown_when_any_required_rectangle_is_missing ... ok
test activity::foreground_helper_tests::activity_idle_math_wraps_with_the_win32_tick_counter ... ok
test activity::foreground_helper_tests::activity_pid_self_exclusion_is_explicit ... ok
test activity::foreground_helper_tests::activity_title_conversion_is_bounded_to_512_utf16_units ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out

running 2 tests
test activity_dpapi_rejects_tampered_ciphertext_without_plaintext_in_error ... ok
test activity_dpapi_round_trips_empty_and_nonempty_payloads ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

### `cargo test -p pw-storage activity`

Exit code: `0`

```text
running 19 tests
test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

The passing suite includes isolated STRICT schema/WAL checks, session paging and mutation, strict retention cutoff behavior, proactive-decision deduplication/rate queries, schema-tamper rejection, future-schema rejection, and plaintext-sentinel checks across the database files.

### `cargo test -p parallel-world-desktop activity`

Exit code: `0`

```text
running 13 tests
test activity_corrupt_settings_fail_closed_before_sampling ... ok
test activity_exclusion_is_case_insensitive_and_precedes_protection_and_storage ... ok
test activity_payload_contains_only_independently_protected_app_and_title ... ok
test activity_pause_forgets_context_and_resume_does_not_merge_across_gap ... ok
test activity_protection_failure_writes_nothing_and_error_hides_plaintext ... ok
test activity_repository_failure_forgets_context_and_never_exposes_raw_values ... ok
test activity_retention_failure_stays_degraded_and_retries_without_losing_sample ... ok
test activity_retention_uses_a_strict_before_cutoff_boundary ... ok
test activity_same_context_compresses_and_changed_context_starts_a_new_session ... ok
test activity_sample_gap_and_clock_reversal_start_fresh_sessions ... ok
test activity_source_is_never_called_when_consent_or_collection_gate_is_off ... ok
test activity_title_protection_failure_after_app_success_still_writes_zero_rows ... ok
test activity_worker_stop_and_drop_join_promptly_without_more_sampling ... ok
test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

### `cargo fmt --all --check`

Exit code: `0`; no output.

### `cargo clippy -p pw-platform -p pw-storage -p parallel-world-desktop --all-targets -- -D warnings`

Exit code: `0`.

```text
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.72s
```

### `git diff --check`

Exit code: `0`. Output contained only Git's existing LF-to-CRLF working-copy warnings; no whitespace errors were reported.

## Additional verification

- `cargo test -p pw-contracts --test context_aware_contracts`: PASS, 6/6.
- `cargo test -p parallel-world-desktop --test behavior_stores`: PASS, 10/10.

## Remaining risks / unverified items

- The installed Rust toolchain has only `x86_64-pc-windows-msvc`, so the non-Windows fallback is guarded and implemented but was not cross-compiled in this environment.
- Tests use deterministic helpers/fakes and DPAPI round trips; they intentionally do not sample the user's real foreground window or require an elevated process. Real desktop startup integration is deferred because IPC/UI/start wiring is outside Task 3.

## Review fix addendum

The Task 3 review found two important gaps. Both were fixed in a follow-up TDD cycle without rewriting the original implementation.

### RED evidence

The expanded collector suite initially ran 17 tests with 5 expected failures:

- collection-disabled settings did not run retention;
- `Ok(None)` and excluded foreground results returned before retention;
- retention degradation was erased by the following healthy `None`/excluded path;
- near-epoch retention produced a negative cutoff;
- Unicode app identifiers such as `ÄPP.EXE` / `äpp.exe` were not matched before protection.

A further focused RED test proved that a simultaneous source failure replaced the still-active retention failure cause in health.

### Fixes

- App-id and title exclusions now share a bounded Unicode-lowercase literal matcher. Inputs are scalar-count bounded before lowercase expansion, ASCII behavior is preserved, and an over-bound comparison fails closed.
- Retention now runs after valid settings and clock checks, before collection consent/source/exclusion decisions. Pending, declined, stale-consent, and collection-disabled states therefore still perform privacy deletion without sampling the foreground source.
- Retention retries on `None`, excluded contexts, and source failures. Successful cleanup establishes the daily cadence; failures remain due until a successful retry.
- A retention degradation flag prevents healthy/disabled paths from erasing the failure. Concurrent source/persistence/protection failures use stable composed messages until retention recovers.
- Retention cutoff arithmetic now saturates at Unix epoch `0`; the repository's strict `< cutoff` behavior continues to retain rows exactly on the cutoff.

### Final follow-up verification

- `cargo test -p pw-platform activity`: PASS; foreground helpers 4/4 and DPAPI tests 2/2.
- `cargo test -p pw-storage activity`: PASS; 19/19.
- `cargo test -p parallel-world-desktop activity`: PASS; 18/18 collector tests.
- `cargo fmt --all --check`: PASS; no output.
- `cargo clippy -p pw-platform -p pw-storage -p parallel-world-desktop --all-targets -- -D warnings`: PASS.
- `git diff --check`: PASS; only existing LF-to-CRLF working-copy warnings were emitted.
