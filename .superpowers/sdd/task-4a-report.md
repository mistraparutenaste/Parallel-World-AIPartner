# Task 4A report: deterministic companion mode resolver

## Status

Implemented the pure Normal/Focus/Night resolver and mode-rule validation within the Task 4A boundary. No Tauri IPC/events, tray/shortcut, LLM, chat/TTS side effects, platform clock adapter, or UI work was added.

## Research

- Confirmed from the current Rust standard-library documentation that `char::to_lowercase` can expand to multiple Unicode scalar values and that `str::chars` iterates Unicode scalar values. The shared bounded normalization therefore extends the complete lowercase iterator for each bounded input character rather than applying ASCII-only folding.
  - https://doc.rust-lang.org/stable/std/primitive.char.html#method.to_lowercase
  - https://doc.rust-lang.org/stable/std/primitive.str.html#method.chars
- Confirmed stable `HashSet::insert` semantics for duplicate detection and retained the existing Serde-derived DTO representation without shape changes.
  - https://doc.rust-lang.org/stable/std/collections/struct.HashSet.html#method.insert
  - https://serde.rs/derive.html

## Changes

- Added `behavior::mode`, with a side-effect-free `ModeResolutionInput`, stable `ModeResolutionError`, exact cloned profile result, and fixed precedence: manual, fullscreen, app, schedule, Normal default.
- Added deterministic same-day and overnight schedule handling with Monday=0, inclusive start, exclusive end, previous-day lookup, and Sunday-to-Monday wrap.
- Added same-tier quietness severity independent of rule order: Night, Focus, Normal.
- Extended `BehaviorSettingsDto::validate` for 32-rule limits, schedule day/time invariants, 1..=64 app ids, bounded/control/empty checks, and Unicode-lowercase duplicate rejection. Disabled rules are validated identically.
- Centralized bounded Unicode app-id normalization in `pw-contracts` and reused it in the Task 3 activity exclusion matcher to prevent ASCII-only or duplicated app matching semantics.

## TDD evidence

### RED

- `cargo test -p pw-contracts behavior_mode_rules`
  - Exit 1 as expected.
  - Both new tests failed because invalid schedule and app activation structures were still accepted.
- `cargo test -p parallel-world-desktop --test mode`
  - Exit 1 as expected.
  - Compilation failed only because `ModeResolutionInput`, `ModeResolutionError`, and `resolve_mode` did not yet exist.

### GREEN and regression

- `cargo test -p pw-contracts behavior_mode_rules`
  - Exit 0; 2 passed.
- `cargo test -p parallel-world-desktop --test mode`
  - Exit 0; 11 passed.
- `cargo test -p parallel-world-desktop activity`
  - Exit 0; Task 3 activity collector suite 18 passed, including Unicode exclusions and fail-closed behavior.

## Acceptance verification

- `cargo test -p pw-contracts context_aware`
  - Exit 0, but Cargo's name filter executed 0 tests because `context_aware` is the integration-test target name rather than a test-function substring.
- `cargo test -p pw-contracts --test context_aware_contracts`
  - Exit 0; 8 passed. Added to provide actual contract-test execution evidence.
- `cargo test -p parallel-world-desktop mode`
  - Exit 0; the new mode suite passed 11/11. The command also matched 8 existing test names outside the new suite; all passed.
- `cargo fmt --all --check`
  - Exit 0.
- `cargo clippy -p pw-contracts -p parallel-world-desktop --all-targets -- -D warnings`
  - Exit 0.
- `git diff --check`
  - Exit 0.

Unrelated Tauri generated schema and permission line-ending churn produced by desktop test builds was restored before commit.

## Unverified / deferred

- Local timezone/DST conversion and reading system foreground/fullscreen state remain intentionally deferred to platform adapters.
- No Tauri runtime integration or visual/manual UI verification applies to this pure resolver task.

## Review addendum

The first review found no production defect but identified insufficient test constraints. The test suite was strengthened without a final production-code change:

- The precedence test now starts with manual, fullscreen, app, and schedule tiers matching simultaneously, then disables each winning tier in order through the Normal default.
- Both app and schedule tiers now prove `Focus > Normal` in forward and reversed rule order, in addition to the existing `Night > Focus` coverage.
- Normal, Focus, and Night profiles now each use distinct boolean combinations and volumes; every manual selection asserts complete profile equality.
- Contract boundary tests now accept exactly 32 schedules, 32 app rules, 64 app ids, and a 260-Unicode-scalar app id. A non-NUL control character is explicitly rejected.

### Review RED evidence

Temporary, uncommitted mutations were applied only to prove the new tests constrain the intended behavior, then fully restored:

- Reordered fullscreen ahead of manual, collapsed Focus severity to Normal, and mapped Normal to the Focus profile.
  - `cargo test -p parallel-world-desktop --test mode` exited 1 with 5 expected failures: full-tier precedence, both Normal/Focus severity tests, default profile, and all-profile selection.
- Reduced the four inclusive limits by one and temporarily validated only NUL rather than all control characters.
  - `cargo test -p pw-contracts --test context_aware_contracts behavior_mode_rules` exited 1 with the 32/32/64/260 boundary tests and non-NUL control rejection failing as expected.

After restoring the original production implementation:

- `cargo test -p parallel-world-desktop --test mode`: exit 0; 13 passed.
- `cargo test -p pw-contracts --test context_aware_contracts behavior_mode_rules`: exit 0; 6 passed.

### Review acceptance verification

- `cargo test -p pw-contracts context_aware`: exit 0; still 0 tests due to the documented name-filter behavior.
- `cargo test -p pw-contracts --test context_aware_contracts`: exit 0; 12 passed.
- `cargo test -p parallel-world-desktop mode`: exit 0; mode suite 13/13 and 8 existing matched tests passed.
- `cargo fmt --all --check`: exit 0.
- `cargo clippy -p pw-contracts -p parallel-world-desktop --all-targets -- -D warnings`: exit 0.
- `git diff --check`: run after generated-churn restoration and report update before the follow-up commit.
