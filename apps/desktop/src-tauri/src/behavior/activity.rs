//! Opt-in foreground activity collection core.

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pw_contracts::{
    ACTIVITY_SESSION_SCHEMA_VERSION, ActivityCollectionHealthEventDto,
    ActivityCollectionHealthStatusDto, BEHAVIOR_SETTINGS_SCHEMA_VERSION, BehaviorSettingsDto,
    ConsentStateDto, ExclusionRuleDto, normalize_activity_app_id,
};
use pw_platform::activity::{
    DataProtector, DpapiProtector, ForegroundContextSource, ForegroundSnapshot,
    SystemForegroundContextSource,
};
use pw_platform::paths::AppDataLayout;
use pw_storage::activity::{ActivityDatabase, NewActivitySession};
use serde::Serialize;
use thiserror::Error;

use super::load_behavior_settings_checked;

const COLLECTION_INTERVAL: Duration = Duration::from_secs(5);
const MAX_CONTIGUOUS_GAP_SECONDS: i64 = 10;
const RETENTION_INTERVAL_SECONDS: i64 = 86_400;
const SECONDS_PER_DAY: i64 = 86_400;
const MAX_EXCLUSION_TITLE_CHARS: usize = 512;
const MAX_EXCLUSION_TITLE_PATTERN_CHARS: usize = 128;
const RETENTION_FAILURE_MESSAGE: &str = "activity retention failed";

pub trait ActivitySettingsSource {
    /// Loads the latest validated behavior settings.
    ///
    /// # Errors
    /// Returns an opaque error when settings are unreadable or invalid.
    fn load(&mut self) -> Result<BehaviorSettingsDto, ActivitySettingsSourceError>;
}

pub trait ActivityRepository {
    /// Persists a new encrypted activity session.
    ///
    /// # Errors
    /// Returns an opaque error when persistence fails.
    fn insert_session(
        &mut self,
        session: &NewActivitySession,
    ) -> Result<i64, ActivityRepositoryError>;

    /// Extends an existing encrypted activity session.
    ///
    /// # Errors
    /// Returns an opaque error when persistence fails.
    fn update_session(
        &mut self,
        id: i64,
        ended_at: Option<i64>,
        duration_seconds: i64,
    ) -> Result<bool, ActivityRepositoryError>;

    /// Deletes activity sessions strictly older than `cutoff`.
    ///
    /// # Errors
    /// Returns an opaque error when retention cleanup fails.
    fn delete_sessions_before(&mut self, cutoff: i64) -> Result<usize, ActivityRepositoryError>;
}

pub trait ActivityClock {
    fn now_unix_seconds(&self) -> i64;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("activity settings source failed")]
pub struct ActivitySettingsSourceError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("activity repository failed")]
pub struct ActivityRepositoryError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ActivityCollectorError {
    #[error("activity settings are unavailable")]
    SettingsUnavailable,
    #[error("activity foreground sampling failed")]
    SourceUnavailable,
    #[error("activity context protection failed")]
    ProtectionFailed,
    #[error("activity persistence failed")]
    PersistenceFailed,
    #[error("activity clock is invalid")]
    InvalidClock,
    #[error("activity payload encoding failed")]
    PayloadEncodingFailed,
}

#[derive(Serialize)]
struct ProtectedContextPayload {
    version: u16,
    protected_app_id: Vec<u8>,
    protected_title: Vec<u8>,
    idle_seconds: u32,
    fullscreen: Option<bool>,
}

struct LastSession {
    id: i64,
    started_at: i64,
    last_sample_at: i64,
    app_id: String,
    title: String,
    category: String,
}

pub struct ActivityCollector<S, F, P, R, C> {
    settings: S,
    source: F,
    protector: P,
    repository: R,
    clock: C,
    last_session: Option<LastSession>,
    last_retention_at: Option<i64>,
    last_retention_days: Option<u16>,
    retention_degraded: bool,
    health: Arc<Mutex<ActivityCollectionHealthEventDto>>,
}

impl<S, F, P, R, C> ActivityCollector<S, F, P, R, C>
where
    S: ActivitySettingsSource,
    F: ForegroundContextSource,
    P: DataProtector,
    R: ActivityRepository,
    C: ActivityClock,
{
    #[must_use]
    pub fn new(settings: S, source: F, protector: P, repository: R, clock: C) -> Self {
        Self {
            settings,
            source,
            protector,
            repository,
            clock,
            last_session: None,
            last_retention_at: None,
            last_retention_days: None,
            retention_degraded: false,
            health: Arc::new(Mutex::new(disabled_health())),
        }
    }

    pub fn source_mut(&mut self) -> &mut F {
        &mut self.source
    }

    #[must_use]
    pub fn health(&self) -> ActivityCollectionHealthEventDto {
        lock_unpoisoned(&self.health).clone()
    }

    /// Performs one gated sample. All failures clear continuity and expose only
    /// stable health/error messages.
    ///
    /// # Errors
    /// Returns a stable collector error for settings, source, protection,
    /// persistence, clock, or payload failures.
    pub fn collect_once(&mut self) -> Result<(), ActivityCollectorError> {
        let settings = self.settings.load().map_err(|_| {
            self.forget_and_degrade("activity settings are unavailable");
            ActivityCollectorError::SettingsUnavailable
        })?;
        if settings.validate().is_err() {
            self.forget_and_degrade("activity settings are invalid");
            return Err(ActivityCollectorError::SettingsUnavailable);
        }
        let now = self.clock.now_unix_seconds();
        if now < 0 {
            self.forget_and_degrade("activity clock is invalid");
            return Err(ActivityCollectorError::InvalidClock);
        }
        self.run_retention_if_due(now, settings.retention_days);
        if !collection_gate_open(&settings) {
            self.last_session = None;
            self.set_disabled();
            return Ok(());
        }
        self.forget_discontinuous(now);

        let snapshot = match self.source.snapshot() {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => {
                self.last_session = None;
                self.set_healthy(None);
                return Ok(());
            }
            Err(_) => {
                self.forget_and_degrade("activity foreground sampling failed");
                return Err(ActivityCollectorError::SourceUnavailable);
            }
        };

        if is_excluded(&snapshot, &settings.exclusions) {
            self.last_session = None;
            self.set_healthy(None);
            return Ok(());
        }

        let category = classify_app(&snapshot.app_id).to_owned();
        if self.extend_if_contiguous(&snapshot, &category, now)? {
            return Ok(());
        }

        let payload = self.protect_payload(&snapshot).inspect_err(|_| {
            self.forget_and_degrade("activity context protection failed");
        })?;
        let session = NewActivitySession {
            started_at: now,
            ended_at: Some(now),
            duration_seconds: 0,
            category: category.clone(),
            payload_version: ACTIVITY_SESSION_SCHEMA_VERSION,
            protected_context: payload,
        };
        let id = self.repository.insert_session(&session).map_err(|_| {
            self.forget_and_degrade("activity persistence failed");
            ActivityCollectorError::PersistenceFailed
        })?;
        self.last_session = Some(LastSession {
            id,
            started_at: now,
            last_sample_at: now,
            app_id: snapshot.app_id,
            title: snapshot.title,
            category,
        });
        self.set_healthy(Some(now));
        Ok(())
    }

    fn extend_if_contiguous(
        &mut self,
        snapshot: &ForegroundSnapshot,
        category: &str,
        now: i64,
    ) -> Result<bool, ActivityCollectorError> {
        let Some(last) = self.last_session.as_ref() else {
            return Ok(false);
        };
        if last.app_id != snapshot.app_id
            || last.title != snapshot.title
            || last.category != category
        {
            return Ok(false);
        }
        let id = last.id;
        let started_at = last.started_at;
        if !self
            .repository
            .update_session(id, Some(now), now - started_at)
            .map_err(|_| {
                self.forget_and_degrade("activity persistence failed");
                ActivityCollectorError::PersistenceFailed
            })?
        {
            self.forget_and_degrade("activity persistence failed");
            return Err(ActivityCollectorError::PersistenceFailed);
        }
        if let Some(last) = self.last_session.as_mut() {
            last.last_sample_at = now;
        }
        self.set_healthy(Some(now));
        Ok(true)
    }

    fn protect_payload(
        &self,
        snapshot: &ForegroundSnapshot,
    ) -> Result<Vec<u8>, ActivityCollectorError> {
        let protected_app_id = self
            .protector
            .protect(snapshot.app_id.as_bytes())
            .map_err(|_| ActivityCollectorError::ProtectionFailed)?;
        let protected_title = self
            .protector
            .protect(snapshot.title.as_bytes())
            .map_err(|_| ActivityCollectorError::ProtectionFailed)?;
        serde_json::to_vec(&ProtectedContextPayload {
            version: ACTIVITY_SESSION_SCHEMA_VERSION,
            protected_app_id,
            protected_title,
            idle_seconds: snapshot.idle_seconds,
            fullscreen: snapshot.fullscreen,
        })
        .map_err(|_| ActivityCollectorError::PayloadEncodingFailed)
    }

    fn forget_discontinuous(&mut self, now: i64) {
        if self.last_session.as_ref().is_some_and(|last| {
            now < last.last_sample_at
                || now.saturating_sub(last.last_sample_at) > MAX_CONTIGUOUS_GAP_SECONDS
        }) {
            self.last_session = None;
        }
    }

    fn run_retention_if_due(&mut self, now: i64, retention_days: u16) {
        let due = self.last_retention_at.is_none_or(|last| {
            now < last || now.saturating_sub(last) >= RETENTION_INTERVAL_SECONDS
        }) || self.last_retention_days != Some(retention_days);
        if !due {
            return;
        }
        let retention_seconds = i64::from(retention_days).saturating_mul(SECONDS_PER_DAY);
        let cutoff = now.saturating_sub(retention_seconds).max(0);
        if self.repository.delete_sessions_before(cutoff).is_err() {
            self.retention_degraded = true;
            self.set_degraded(RETENTION_FAILURE_MESSAGE);
            return;
        }
        self.last_retention_at = Some(now);
        self.last_retention_days = Some(retention_days);
        self.retention_degraded = false;
    }

    fn forget_and_degrade(&mut self, message: &'static str) {
        self.last_session = None;
        self.set_degraded(message);
    }

    fn set_healthy(&self, last_activity_at: Option<i64>) {
        if self.retention_degraded {
            let previous_activity_at = lock_unpoisoned(&self.health).last_activity_at;
            self.set_health(ActivityCollectionHealthEventDto {
                schema_version: BEHAVIOR_SETTINGS_SCHEMA_VERSION,
                status: ActivityCollectionHealthStatusDto::Degraded,
                last_activity_at: last_activity_at.or(previous_activity_at),
                message: Some(RETENTION_FAILURE_MESSAGE.to_owned()),
            });
            return;
        }
        self.set_health(ActivityCollectionHealthEventDto {
            schema_version: BEHAVIOR_SETTINGS_SCHEMA_VERSION,
            status: ActivityCollectionHealthStatusDto::Healthy,
            last_activity_at,
            message: None,
        });
    }

    fn set_disabled(&self) {
        if self.retention_degraded {
            self.set_degraded(RETENTION_FAILURE_MESSAGE);
        } else {
            self.set_health(disabled_health());
        }
    }

    fn set_degraded(&self, message: &'static str) {
        let last_activity_at = lock_unpoisoned(&self.health).last_activity_at;
        let message = if self.retention_degraded && message != RETENTION_FAILURE_MESSAGE {
            format!("{RETENTION_FAILURE_MESSAGE}; {message}")
        } else {
            message.to_owned()
        };
        self.set_health(ActivityCollectionHealthEventDto {
            schema_version: BEHAVIOR_SETTINGS_SCHEMA_VERSION,
            status: ActivityCollectionHealthStatusDto::Degraded,
            last_activity_at,
            message: Some(message),
        });
    }

    fn set_health(&self, health: ActivityCollectionHealthEventDto) {
        *lock_unpoisoned(&self.health) = health;
    }
}

fn collection_gate_open(settings: &BehaviorSettingsDto) -> bool {
    settings.consent == ConsentStateDto::Accepted
        && settings.consent_version == 1
        && settings.collection_enabled
}

fn is_excluded(snapshot: &ForegroundSnapshot, exclusions: &[ExclusionRuleDto]) -> bool {
    exclusions.iter().any(|rule| {
        let app_matches = rule.app_id.as_ref().is_none_or(|app_id| {
            normalize_activity_app_id(&snapshot.app_id)
                .zip(normalize_activity_app_id(app_id))
                .is_none_or(|(value, selector)| value == selector)
        });
        let title_matches = rule.title_pattern.as_ref().is_none_or(|pattern| {
            bounded_unicode_literal_match(
                &snapshot.title,
                pattern,
                MAX_EXCLUSION_TITLE_CHARS,
                MAX_EXCLUSION_TITLE_PATTERN_CHARS,
            )
            .unwrap_or(true)
        });
        app_matches && title_matches
    })
}

fn bounded_unicode_literal_match(
    value: &str,
    selector: &str,
    max_value_chars: usize,
    max_selector_chars: usize,
) -> Option<bool> {
    let value = bounded_unicode_lowercase(value, max_value_chars)?;
    let selector = bounded_unicode_lowercase(selector, max_selector_chars)?;
    Some(value.contains(&selector))
}

fn bounded_unicode_lowercase(value: &str, max_chars: usize) -> Option<String> {
    let mut chars = value.chars();
    let mut lowercase = String::with_capacity(max_chars.saturating_mul(4));
    for _ in 0..max_chars {
        let Some(character) = chars.next() else {
            return Some(lowercase);
        };
        lowercase.extend(character.to_lowercase());
    }
    chars.next().is_none().then_some(lowercase)
}

fn classify_app(app_id: &str) -> &'static str {
    match app_id.to_ascii_lowercase().as_str() {
        "code.exe"
        | "devenv.exe"
        | "idea64.exe"
        | "rider64.exe"
        | "terminal.exe"
        | "windowsterminal.exe" => "development",
        "chrome.exe" | "firefox.exe" | "msedge.exe" | "opera.exe" | "brave.exe" => "browser",
        "discord.exe" | "slack.exe" | "teams.exe" | "zoom.exe" => "communication",
        "blender.exe" | "photoshop.exe" | "illustrator.exe" | "krita.exe" => "creative",
        _ => "other",
    }
}

fn disabled_health() -> ActivityCollectionHealthEventDto {
    ActivityCollectionHealthEventDto {
        schema_version: BEHAVIOR_SETTINGS_SCHEMA_VERSION,
        status: ActivityCollectionHealthStatusDto::Disabled,
        last_activity_at: None,
        message: None,
    }
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl ActivityRepository for ActivityDatabase {
    fn insert_session(
        &mut self,
        session: &NewActivitySession,
    ) -> Result<i64, ActivityRepositoryError> {
        ActivityDatabase::insert_session(self, session).map_err(|_| ActivityRepositoryError)
    }

    fn update_session(
        &mut self,
        id: i64,
        ended_at: Option<i64>,
        duration_seconds: i64,
    ) -> Result<bool, ActivityRepositoryError> {
        ActivityDatabase::update_session(self, id, ended_at, duration_seconds)
            .map_err(|_| ActivityRepositoryError)
    }

    fn delete_sessions_before(&mut self, cutoff: i64) -> Result<usize, ActivityRepositoryError> {
        ActivityDatabase::delete_sessions_before(self, cutoff).map_err(|_| ActivityRepositoryError)
    }
}

struct FileActivitySettingsSource {
    layout: AppDataLayout,
}

impl ActivitySettingsSource for FileActivitySettingsSource {
    fn load(&mut self) -> Result<BehaviorSettingsDto, ActivitySettingsSourceError> {
        load_behavior_settings_checked(&self.layout).map_err(|_| ActivitySettingsSourceError)
    }
}

#[derive(Debug, Clone, Copy)]
struct SystemClock;

impl ActivityClock for SystemClock {
    fn now_unix_seconds(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| i64::try_from(duration.as_secs()).ok())
            .unwrap_or(-1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ActivityCollectorStartError {
    #[error("activity database could not be opened")]
    Database,
    #[error("activity collector worker could not be started")]
    Worker,
}

pub struct ActivityCollectorService {
    stop: Mutex<Option<Sender<()>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    health: Arc<Mutex<ActivityCollectionHealthEventDto>>,
}

impl ActivityCollectorService {
    /// Starts the production collector. The `SQLite` connection is opened before
    /// spawning, then moved into and exclusively owned by the worker thread.
    ///
    /// # Errors
    /// Returns a stable error if the dedicated activity database or worker fails.
    pub fn start(layout: &AppDataLayout) -> Result<Self, ActivityCollectorStartError> {
        let repository = ActivityDatabase::open(layout.activity_database())
            .map_err(|_| ActivityCollectorStartError::Database)?;
        let collector = ActivityCollector::new(
            FileActivitySettingsSource {
                layout: layout.clone(),
            },
            SystemForegroundContextSource::default(),
            DpapiProtector,
            repository,
            SystemClock,
        );
        Self::start_with_interval(collector, COLLECTION_INTERVAL)
    }

    /// Starts a collector worker with an injectable interval for deterministic tests.
    ///
    /// # Errors
    /// Returns a stable error if the worker thread cannot be started.
    pub fn start_with_interval<S, F, P, R, C>(
        mut collector: ActivityCollector<S, F, P, R, C>,
        interval: Duration,
    ) -> Result<Self, ActivityCollectorStartError>
    where
        S: ActivitySettingsSource + Send + 'static,
        F: ForegroundContextSource + Send + 'static,
        P: DataProtector + Send + 'static,
        R: ActivityRepository + Send + 'static,
        C: ActivityClock + Send + 'static,
    {
        let health = Arc::clone(&collector.health);
        let (stop, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("activity-collector".to_owned())
            .spawn(move || {
                while let Err(RecvTimeoutError::Timeout) = receiver.recv_timeout(interval) {
                    let _ = collector.collect_once();
                }
            })
            .map_err(|_| ActivityCollectorStartError::Worker)?;
        Ok(Self {
            stop: Mutex::new(Some(stop)),
            worker: Mutex::new(Some(worker)),
            health,
        })
    }

    #[must_use]
    pub fn health(&self) -> ActivityCollectionHealthEventDto {
        lock_unpoisoned(&self.health).clone()
    }

    pub fn stop(&self) {
        if let Some(stop) = lock_unpoisoned(&self.stop).take() {
            let _ = stop.send(());
        }
        let worker = lock_unpoisoned(&self.worker).take();
        if let Some(worker) = worker {
            let _ = worker.join();
        }
    }
}

impl Drop for ActivityCollectorService {
    fn drop(&mut self) {
        self.stop();
    }
}
