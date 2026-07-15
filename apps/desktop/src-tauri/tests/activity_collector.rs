use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use parallel_world_desktop::behavior::{
    ActivityClock, ActivityCollector, ActivityCollectorService, ActivityRepository,
    ActivityRepositoryError, ActivitySettingsSource, ActivitySettingsSourceError,
};
use pw_contracts::{
    ActivityCollectionHealthStatusDto, BehaviorSettingsDto, ConsentStateDto, ExclusionRuleDto,
};
use pw_platform::activity::{
    DataProtectionError, DataProtector, ForegroundContextError, ForegroundContextSource,
    ForegroundSnapshot,
};
use pw_storage::activity::NewActivitySession;

#[derive(Clone)]
struct FakeSettings(Arc<Mutex<Result<BehaviorSettingsDto, ()>>>);

impl ActivitySettingsSource for FakeSettings {
    fn load(&mut self) -> Result<BehaviorSettingsDto, ActivitySettingsSourceError> {
        self.0
            .lock()
            .unwrap()
            .clone()
            .map_err(|()| ActivitySettingsSourceError)
    }
}

struct FakeSource {
    calls: Arc<AtomicUsize>,
    snapshot: ForegroundSnapshot,
}

impl ForegroundContextSource for FakeSource {
    fn snapshot(&mut self) -> Result<Option<ForegroundSnapshot>, ForegroundContextError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Some(self.snapshot.clone()))
    }
}

#[derive(Clone)]
struct FakeProtector {
    calls: Arc<AtomicUsize>,
    fail_on_call: Option<usize>,
}

impl DataProtector for FakeProtector {
    fn protect(&self, plaintext: &[u8]) -> Result<Vec<u8>, DataProtectionError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if self.fail_on_call == Some(call) {
            Err(DataProtectionError::UnsupportedPlatform)
        } else {
            let mut ciphertext = b"cipher:".to_vec();
            ciphertext.extend(plaintext.iter().map(|byte| byte ^ 0xA5));
            Ok(ciphertext)
        }
    }

    fn unprotect(&self, _protected: &[u8]) -> Result<Vec<u8>, DataProtectionError> {
        unreachable!("collector never decrypts")
    }
}

#[derive(Default)]
struct RepoState {
    inserted: Vec<NewActivitySession>,
    updated: Vec<(i64, Option<i64>, i64)>,
    cutoffs: Vec<i64>,
    fail_insert: bool,
    fail_retention: bool,
}

struct FakeRepository(Arc<Mutex<RepoState>>);

impl ActivityRepository for FakeRepository {
    fn insert_session(
        &mut self,
        session: &NewActivitySession,
    ) -> Result<i64, ActivityRepositoryError> {
        let mut state = self.0.lock().unwrap();
        if state.fail_insert {
            return Err(ActivityRepositoryError);
        }
        state.inserted.push(session.clone());
        Ok(i64::try_from(state.inserted.len()).expect("test session count fits i64"))
    }

    fn update_session(
        &mut self,
        id: i64,
        ended_at: Option<i64>,
        duration_seconds: i64,
    ) -> Result<bool, ActivityRepositoryError> {
        self.0
            .lock()
            .unwrap()
            .updated
            .push((id, ended_at, duration_seconds));
        Ok(true)
    }

    fn delete_sessions_before(&mut self, cutoff: i64) -> Result<usize, ActivityRepositoryError> {
        let mut state = self.0.lock().unwrap();
        state.cutoffs.push(cutoff);
        if state.fail_retention {
            return Err(ActivityRepositoryError);
        }
        Ok(0)
    }
}

#[derive(Clone)]
struct FakeClock(Arc<Mutex<i64>>);

impl ActivityClock for FakeClock {
    fn now_unix_seconds(&self) -> i64 {
        *self.0.lock().unwrap()
    }
}

fn enabled_settings() -> BehaviorSettingsDto {
    BehaviorSettingsDto {
        consent: ConsentStateDto::Accepted,
        consent_version: 1,
        collection_enabled: true,
        ..BehaviorSettingsDto::default()
    }
}

fn snapshot(app_id: &str, title: &str) -> ForegroundSnapshot {
    ForegroundSnapshot {
        app_id: app_id.to_owned(),
        title: title.to_owned(),
        idle_seconds: 3,
        fullscreen: Some(false),
    }
}

fn collector(
    settings: Arc<Mutex<Result<BehaviorSettingsDto, ()>>>,
    source_calls: Arc<AtomicUsize>,
    protector_calls: Arc<AtomicUsize>,
    repo: Arc<Mutex<RepoState>>,
    now: Arc<Mutex<i64>>,
    foreground: ForegroundSnapshot,
    protector_fails: bool,
) -> ActivityCollector<FakeSettings, FakeSource, FakeProtector, FakeRepository, FakeClock> {
    ActivityCollector::new(
        FakeSettings(settings),
        FakeSource {
            calls: source_calls,
            snapshot: foreground,
        },
        FakeProtector {
            calls: protector_calls,
            fail_on_call: protector_fails.then_some(1),
        },
        FakeRepository(repo),
        FakeClock(now),
    )
}

#[test]
fn activity_source_is_never_called_when_consent_or_collection_gate_is_off() {
    for settings in [
        BehaviorSettingsDto::default(),
        {
            let mut value = enabled_settings();
            value.collection_enabled = false;
            value
        },
        {
            let mut value = enabled_settings();
            value.consent_version = 2;
            value
        },
    ] {
        let source_calls = Arc::new(AtomicUsize::new(0));
        let mut collector = collector(
            Arc::new(Mutex::new(Ok(settings))),
            Arc::clone(&source_calls),
            Arc::new(AtomicUsize::new(0)),
            Arc::new(Mutex::new(RepoState::default())),
            Arc::new(Mutex::new(1_000_000)),
            snapshot("code.exe", "workspace"),
            false,
        );

        collector.collect_once().expect("gate is a safe no-op");
        assert_eq!(source_calls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn activity_corrupt_settings_fail_closed_before_sampling() {
    let source_calls = Arc::new(AtomicUsize::new(0));
    let mut collector = collector(
        Arc::new(Mutex::new(Err(()))),
        Arc::clone(&source_calls),
        Arc::new(AtomicUsize::new(0)),
        Arc::new(Mutex::new(RepoState::default())),
        Arc::new(Mutex::new(1_000_000)),
        snapshot("code.exe", "workspace"),
        false,
    );

    assert!(collector.collect_once().is_err());
    assert_eq!(source_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn activity_exclusion_is_case_insensitive_and_precedes_protection_and_storage() {
    let mut settings = enabled_settings();
    settings.exclusions.push(ExclusionRuleDto {
        app_id: Some("CODE.EXE".to_owned()),
        title_pattern: Some("private project".to_owned()),
    });
    let protector_calls = Arc::new(AtomicUsize::new(0));
    let repo = Arc::new(Mutex::new(RepoState::default()));
    let mut collector = collector(
        Arc::new(Mutex::new(Ok(settings))),
        Arc::new(AtomicUsize::new(0)),
        Arc::clone(&protector_calls),
        Arc::clone(&repo),
        Arc::new(Mutex::new(1_000_000)),
        snapshot("code.exe", "My PRIVATE PROJECT notes"),
        false,
    );

    collector
        .collect_once()
        .expect("excluded context is skipped");
    assert_eq!(protector_calls.load(Ordering::SeqCst), 0);
    assert!(repo.lock().unwrap().inserted.is_empty());
}

#[test]
fn activity_protection_failure_writes_nothing_and_error_hides_plaintext() {
    const SECRET_APP: &str = "secret-app.exe";
    const SECRET_TITLE: &str = "private title sentinel";
    let repo = Arc::new(Mutex::new(RepoState::default()));
    let mut collector = collector(
        Arc::new(Mutex::new(Ok(enabled_settings()))),
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicUsize::new(0)),
        Arc::clone(&repo),
        Arc::new(Mutex::new(1_000_000)),
        snapshot(SECRET_APP, SECRET_TITLE),
        true,
    );

    let error = collector
        .collect_once()
        .expect_err("protection fails closed");
    let display = error.to_string();
    assert!(!display.contains(SECRET_APP));
    assert!(!display.contains(SECRET_TITLE));
    assert!(repo.lock().unwrap().inserted.is_empty());
}

#[test]
fn activity_payload_contains_only_independently_protected_app_and_title() {
    const SECRET_APP: &str = "raw-app.exe";
    const SECRET_TITLE: &str = "raw title";
    let protector_calls = Arc::new(AtomicUsize::new(0));
    let repo = Arc::new(Mutex::new(RepoState::default()));
    let mut collector = collector(
        Arc::new(Mutex::new(Ok(enabled_settings()))),
        Arc::new(AtomicUsize::new(0)),
        Arc::clone(&protector_calls),
        Arc::clone(&repo),
        Arc::new(Mutex::new(1_000_000)),
        snapshot(SECRET_APP, SECRET_TITLE),
        false,
    );

    collector.collect_once().expect("collect protected session");
    assert_eq!(protector_calls.load(Ordering::SeqCst), 2);
    let state = repo.lock().unwrap();
    let payload = &state.inserted[0].protected_context;
    assert!(
        !payload
            .windows(SECRET_APP.len())
            .any(|w| w == SECRET_APP.as_bytes())
    );
    assert!(
        !payload
            .windows(SECRET_TITLE.len())
            .any(|w| w == SECRET_TITLE.as_bytes())
    );
    let json: serde_json::Value = serde_json::from_slice(payload).expect("versioned payload json");
    assert_eq!(json["version"], 1);
    assert!(json["protected_app_id"].is_array());
    assert!(json["protected_title"].is_array());
}

#[test]
fn activity_title_protection_failure_after_app_success_still_writes_zero_rows() {
    let calls = Arc::new(AtomicUsize::new(0));
    let repo = Arc::new(Mutex::new(RepoState::default()));
    let mut collector = ActivityCollector::new(
        FakeSettings(Arc::new(Mutex::new(Ok(enabled_settings())))),
        FakeSource {
            calls: Arc::new(AtomicUsize::new(0)),
            snapshot: snapshot("private-app.exe", "private-title"),
        },
        FakeProtector {
            calls: Arc::clone(&calls),
            fail_on_call: Some(2),
        },
        FakeRepository(Arc::clone(&repo)),
        FakeClock(Arc::new(Mutex::new(1_000_000))),
    );

    let error = collector
        .collect_once()
        .expect_err("title protection fails");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(error.to_string(), "activity context protection failed");
    assert!(repo.lock().unwrap().inserted.is_empty());
}

#[test]
fn activity_repository_failure_forgets_context_and_never_exposes_raw_values() {
    const SECRET_APP: &str = "private-repository-app.exe";
    const SECRET_TITLE: &str = "private repository title";
    let repo = Arc::new(Mutex::new(RepoState {
        fail_insert: true,
        ..RepoState::default()
    }));
    let mut collector = collector(
        Arc::new(Mutex::new(Ok(enabled_settings()))),
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicUsize::new(0)),
        Arc::clone(&repo),
        Arc::new(Mutex::new(1_000_000)),
        snapshot(SECRET_APP, SECRET_TITLE),
        false,
    );

    let error = collector
        .collect_once()
        .expect_err("repository fails closed");
    for formatted in [error.to_string(), format!("{error:?}")] {
        assert!(!formatted.contains(SECRET_APP));
        assert!(!formatted.contains(SECRET_TITLE));
    }
    assert!(repo.lock().unwrap().inserted.is_empty());
}

#[test]
fn activity_same_context_compresses_and_changed_context_starts_a_new_session() {
    let settings = Arc::new(Mutex::new(Ok(enabled_settings())));
    let repo = Arc::new(Mutex::new(RepoState::default()));
    let now = Arc::new(Mutex::new(1_000_000));
    let mut collector = collector(
        settings,
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicUsize::new(0)),
        Arc::clone(&repo),
        Arc::clone(&now),
        snapshot("code.exe", "workspace"),
        false,
    );

    collector.collect_once().unwrap();
    *now.lock().unwrap() += 5;
    collector.collect_once().unwrap();
    collector.source_mut().snapshot = snapshot("firefox.exe", "docs");
    *now.lock().unwrap() += 5;
    collector.collect_once().unwrap();

    let state = repo.lock().unwrap();
    assert_eq!(state.inserted.len(), 2);
    assert_eq!(state.updated, vec![(1, Some(1_000_005), 5)]);
    assert_eq!(state.inserted[0].category, "development");
    assert_eq!(state.inserted[1].category, "browser");
}

#[test]
fn activity_pause_forgets_context_and_resume_does_not_merge_across_gap() {
    let settings = Arc::new(Mutex::new(Ok(enabled_settings())));
    let repo = Arc::new(Mutex::new(RepoState::default()));
    let now = Arc::new(Mutex::new(1_000_000));
    let mut collector = collector(
        Arc::clone(&settings),
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicUsize::new(0)),
        Arc::clone(&repo),
        Arc::clone(&now),
        snapshot("code.exe", "workspace"),
        false,
    );

    collector.collect_once().unwrap();
    settings
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .collection_enabled = false;
    *now.lock().unwrap() += 300;
    collector.collect_once().unwrap();
    settings
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .collection_enabled = true;
    collector.collect_once().unwrap();

    let state = repo.lock().unwrap();
    assert_eq!(state.inserted.len(), 2);
    assert!(state.updated.is_empty());
}

#[test]
fn activity_sample_gap_and_clock_reversal_start_fresh_sessions() {
    for second_time in [1_000_011, 999_999] {
        let repo = Arc::new(Mutex::new(RepoState::default()));
        let now = Arc::new(Mutex::new(1_000_000));
        let mut collector = collector(
            Arc::new(Mutex::new(Ok(enabled_settings()))),
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
            Arc::clone(&repo),
            Arc::clone(&now),
            snapshot("code.exe", "workspace"),
            false,
        );
        collector.collect_once().unwrap();
        *now.lock().unwrap() = second_time;
        collector.collect_once().unwrap();
        assert_eq!(repo.lock().unwrap().inserted.len(), 2);
    }
}

#[test]
fn activity_retention_uses_a_strict_before_cutoff_boundary() {
    let repo = Arc::new(Mutex::new(RepoState::default()));
    let now = Arc::new(Mutex::new(3_000_000));
    let mut settings = enabled_settings();
    settings.retention_days = 30;
    let mut collector = collector(
        Arc::new(Mutex::new(Ok(settings))),
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicUsize::new(0)),
        Arc::clone(&repo),
        now,
        snapshot("code.exe", "workspace"),
        false,
    );

    collector.collect_once().unwrap();
    assert_eq!(repo.lock().unwrap().cutoffs, vec![3_000_000 - 30 * 86_400]);
}

#[test]
fn activity_retention_failure_stays_degraded_and_retries_without_losing_sample() {
    let repo = Arc::new(Mutex::new(RepoState {
        fail_retention: true,
        ..RepoState::default()
    }));
    let mut collector = collector(
        Arc::new(Mutex::new(Ok(enabled_settings()))),
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicUsize::new(0)),
        Arc::clone(&repo),
        Arc::new(Mutex::new(3_000_000)),
        snapshot("code.exe", "workspace"),
        false,
    );

    collector
        .collect_once()
        .expect("sample survives retention failure");
    assert_eq!(repo.lock().unwrap().inserted.len(), 1);
    assert_eq!(
        collector.health().status,
        ActivityCollectionHealthStatusDto::Degraded
    );
    assert_eq!(
        collector.health().message.as_deref(),
        Some("activity retention failed")
    );
}

#[test]
fn activity_worker_stop_and_drop_join_promptly_without_more_sampling() {
    let source_calls = Arc::new(AtomicUsize::new(0));
    let first_collector = collector(
        Arc::new(Mutex::new(Ok(enabled_settings()))),
        Arc::clone(&source_calls),
        Arc::new(AtomicUsize::new(0)),
        Arc::new(Mutex::new(RepoState::default())),
        Arc::new(Mutex::new(1_000_000)),
        snapshot("code.exe", "workspace"),
        false,
    );
    let service =
        ActivityCollectorService::start_with_interval(first_collector, Duration::from_millis(10))
            .expect("start worker");
    let deadline = Instant::now() + Duration::from_secs(1);
    while source_calls.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(source_calls.load(Ordering::SeqCst) > 0);

    let stopped_at = Instant::now();
    service.stop();
    assert!(stopped_at.elapsed() < Duration::from_millis(250));
    let calls_after_stop = source_calls.load(Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(source_calls.load(Ordering::SeqCst), calls_after_stop);

    let collector = collector(
        Arc::new(Mutex::new(Ok(enabled_settings()))),
        Arc::clone(&source_calls),
        Arc::new(AtomicUsize::new(0)),
        Arc::new(Mutex::new(RepoState::default())),
        Arc::new(Mutex::new(1_000_000)),
        snapshot("code.exe", "workspace"),
        false,
    );
    let dropped_at = Instant::now();
    drop(ActivityCollectorService::start_with_interval(
        collector,
        Duration::from_secs(5),
    ));
    assert!(dropped_at.elapsed() < Duration::from_millis(250));
}
