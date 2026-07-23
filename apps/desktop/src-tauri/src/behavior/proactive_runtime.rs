//! Fail-closed proactive delivery policy and desktop worker adapters.

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pw_application::behavior::proactive::{
    Candidate, CandidateEngine, CandidateKind, CategoryId, FrequencyPolicy, InteractionGate,
    Observation, ProactiveGatePolicy, ProactiveThresholds, with_proactive_turn,
};
use pw_application::history::{ProactiveAssistantHistory, ProactiveAssistantMessage};
use pw_application::memory::CompanionStateStore;
use pw_contracts::{
    ACTIVE_MODE_CHANGED_EVENT, ActiveModeChangedEventDto, ActiveModeDto,
    ActivityCollectionHealthEventDto, BehaviorSettingsDto, ChatMessageEventDto, ChatRoleDto,
    SCHEMA_VERSION,
};
use pw_domain::reply::TurnTracker;
use pw_platform::activity::{ForegroundContextSource, SystemForegroundContextSource};
use pw_platform::paths::AppDataLayout;
use pw_storage::activity::{
    ActivityDatabase, FinalSpeakDecisionOutcome, FinalSpeakDecisionRequest,
};
use pw_storage::{Database, SqliteCompanionStateStore, SqliteConversationHistory};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use time::OffsetDateTime;

use crate::chat::ChatService;

use super::{
    ActivityCollectorService, ModeResolutionInput, load_behavior_settings_checked, resolve_mode,
};

const MAX_PROACTIVE_TEXT_CHARS: usize = 500;
const RUNTIME_INTERVAL: Duration = Duration::from_secs(5);
const DEFAULT_CONVERSATION_ID: &str = "default";

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct ProactiveDeliveryInput {
    pub master_enabled: bool,
    pub profile_enabled: bool,
    pub trigger_enabled: bool,
    pub in_quiet_hours: bool,
    pub temporary_conversation: bool,
    pub evaluator_approved: bool,
    pub generated_text: String,
    pub lease_cancelled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProactiveDeliveryDecision {
    Deliver(String),
    Skip,
}

#[must_use]
pub fn decide_proactive_delivery(input: &ProactiveDeliveryInput) -> ProactiveDeliveryDecision {
    let text = input.generated_text.trim();
    if !input.master_enabled
        || !input.profile_enabled
        || !input.trigger_enabled
        || input.in_quiet_hours
        || input.temporary_conversation
        || !input.evaluator_approved
        || input.lease_cancelled
        || text.is_empty()
        || text.chars().count() > MAX_PROACTIVE_TEXT_CHARS
    {
        return ProactiveDeliveryDecision::Skip;
    }
    ProactiveDeliveryDecision::Deliver(text.to_owned())
}

pub struct BehaviorRuntimeService {
    collector: ActivityCollectorService,
    stop: Mutex<Option<Sender<()>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    active_mode: Arc<Mutex<Option<ActiveModeDto>>>,
}

impl BehaviorRuntimeService {
    /// Starts activity collection and the proactive behavior worker.
    ///
    /// # Errors
    ///
    /// Returns an error when the encrypted activity store or either background
    /// worker cannot be initialized.
    pub fn start<R: Runtime>(
        app: AppHandle<R>,
        layout: &AppDataLayout,
        gate: Arc<InteractionGate>,
    ) -> Result<Self, String> {
        let collector =
            ActivityCollectorService::start(layout).map_err(|error| error.to_string())?;
        let active_mode = Arc::new(Mutex::new(None));
        let mode_state = Arc::clone(&active_mode);
        let runtime_layout = layout.clone();
        let (stop, receiver) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("proactive-runtime".to_owned())
            .spawn(move || {
                let mut thresholds = ProactiveThresholds::default();
                let mut engine = CandidateEngine::new(thresholds);
                let mut source = SystemForegroundContextSource::default();
                loop {
                    if let Err(error) = runtime_tick(
                        &app,
                        &runtime_layout,
                        &gate,
                        &mut engine,
                        &mut thresholds,
                        &mut source,
                        &mode_state,
                    ) {
                        tracing::warn!(%error, "proactive runtime tick skipped");
                    }
                    match receiver.recv_timeout(RUNTIME_INTERVAL) {
                        Err(RecvTimeoutError::Timeout) => {}
                        Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            collector,
            stop: Mutex::new(Some(stop)),
            worker: Mutex::new(Some(worker)),
            active_mode,
        })
    }

    #[must_use]
    pub fn collection_health(&self) -> ActivityCollectionHealthEventDto {
        self.collector.health()
    }

    #[must_use]
    pub fn active_mode(&self) -> Option<ActiveModeDto> {
        self.active_mode
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn stop(&self) {
        if let Some(stop) = self
            .stop
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _ = stop.send(());
        }
        if let Some(worker) = self
            .worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _ = worker.join();
        }
        self.collector.stop();
    }
}

impl Drop for BehaviorRuntimeService {
    fn drop(&mut self) {
        self.stop();
    }
}

fn runtime_tick<R: Runtime>(
    app: &AppHandle<R>,
    layout: &AppDataLayout,
    gate: &InteractionGate,
    engine: &mut CandidateEngine,
    current_thresholds: &mut ProactiveThresholds,
    source: &mut SystemForegroundContextSource,
    mode_state: &Mutex<Option<ActiveModeDto>>,
) -> Result<(), String> {
    let settings = load_behavior_settings_checked(layout).map_err(|error| error.to_string())?;
    let thresholds = proactive_thresholds(&settings);
    if thresholds != *current_thresholds {
        *current_thresholds = thresholds;
        *engine = CandidateEngine::new(thresholds);
    }
    let now = unix_timestamp();
    let local = OffsetDateTime::now_local().map_err(|_| "local time is unavailable")?;
    let foreground = source.snapshot().ok().flatten();
    let resolved = resolve_mode(
        &settings,
        &ModeResolutionInput {
            local_weekday: local.weekday().number_days_from_monday(),
            local_minutes: u16::from(local.hour()) * 60 + u16::from(local.minute()),
            foreground_app_id: foreground.as_ref().map(|value| value.app_id.clone()),
            fullscreen: foreground.as_ref().and_then(|value| value.fullscreen),
        },
    )
    .map_err(|error| error.to_string())?;
    publish_mode_if_changed(app, mode_state, &resolved.active_mode);

    let database_path = layout.data.join("activity.sqlite3");
    let history = ActivityDatabase::open(&database_path).map_err(|error| error.to_string())?;
    let page = history
        .page_sessions(1, None)
        .map_err(|error| error.to_string())?;
    let Some(session) = page.sessions.first() else {
        return Ok(());
    };
    let observation = Observation::new(
        u64::try_from(session.id).map_err(|_| "activity id is invalid")?,
        session.started_at,
        session.ended_at.unwrap_or(session.started_at),
        CategoryId::new(&session.category).map_err(|_| "activity category is invalid")?,
    );
    let Some(candidate) = engine.observe_checked(observation) else {
        return Ok(());
    };
    if !trigger_enabled(&settings, candidate.kind()) || in_quiet_hours(&settings, &local) {
        return Ok(());
    }
    let temporary = Database::open(layout.data.join("parallel-world.sqlite3"))
        .ok()
        .map(SqliteCompanionStateStore::new)
        .and_then(|store| {
            store
                .is_temporary_conversation(DEFAULT_CONVERSATION_ID)
                .ok()
        })
        .unwrap_or(true);
    let policy = ProactiveGatePolicy {
        master_enabled: settings.proactive_master_enabled,
        profile_enabled: resolved.profile.proactive_enabled,
        snoozed_until: settings.proactive_snoozed_until,
        temporary_conversation: temporary,
        policy_error: false,
        now,
        frequency: frequency_policy(&settings),
    };
    let _ = with_proactive_turn(gate, &history, &candidate, policy, |lease| {
        let cue = candidate_cue(&candidate);
        let generated = app
            .state::<ChatService>()
            .generate_proactive_reply(app, &cue)
            .ok();
        let Some(generated) = generated else {
            return;
        };
        let decision = decide_proactive_delivery(&ProactiveDeliveryInput {
            master_enabled: settings.proactive_master_enabled,
            profile_enabled: resolved.profile.proactive_enabled,
            trigger_enabled: true,
            in_quiet_hours: false,
            temporary_conversation: temporary,
            evaluator_approved: evaluator_approves(app, &settings, &cue, &generated),
            generated_text: generated,
            lease_cancelled: lease.is_cancelled(),
        });
        let ProactiveDeliveryDecision::Deliver(text) = decision else {
            return;
        };
        if lease.is_cancelled() {
            return;
        }
        if let Err(error) = persist_and_deliver(
            app,
            layout,
            &candidate,
            &text,
            now,
            &settings,
            resolved.profile.tts_enabled,
        ) {
            tracing::warn!(%error, "proactive delivery skipped");
        }
    });
    Ok(())
}

fn publish_mode_if_changed<R: Runtime>(
    app: &AppHandle<R>,
    state: &Mutex<Option<ActiveModeDto>>,
    active_mode: &ActiveModeDto,
) {
    let mut current = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if current.as_ref() == Some(active_mode) {
        return;
    }
    *current = Some(active_mode.clone());
    let _ = app.emit(
        ACTIVE_MODE_CHANGED_EVENT,
        ActiveModeChangedEventDto {
            schema_version: active_mode.schema_version,
            active_mode: active_mode.clone(),
        },
    );
}

fn persist_and_deliver<R: Runtime>(
    app: &AppHandle<R>,
    layout: &AppDataLayout,
    candidate: &Candidate,
    text: &str,
    now: i64,
    settings: &BehaviorSettingsDto,
    tts_enabled: bool,
) -> Result<(), String> {
    let mut activity = ActivityDatabase::open(layout.data.join("activity.sqlite3"))
        .map_err(|error| error.to_string())?;
    let frequency = frequency_policy(settings);
    let outcome = activity
        .record_final_speak(FinalSpeakDecisionRequest {
            created_at: now,
            candidate_kind: candidate_kind_name(candidate.kind()),
            topic_hash: candidate.topic_hash(),
            minimum_interval_seconds: frequency.minimum_interval,
            hour_since: now.saturating_sub(3_599),
            day_since: now.saturating_sub(86_399),
            max_per_hour: frequency.max_per_hour,
            max_per_day: frequency.max_per_day,
        })
        .map_err(|error| error.to_string())?;
    if !matches!(outcome, FinalSpeakDecisionOutcome::Inserted { .. }) {
        return Ok(());
    }
    let database = Database::open(layout.data.join("parallel-world.sqlite3"))
        .map_err(|error| error.to_string())?;
    let mut history = SqliteConversationHistory::new(database);
    let persisted = history
        .append_proactive_assistant(&ProactiveAssistantMessage {
            conversation_id: DEFAULT_CONVERSATION_ID.to_owned(),
            content: text.to_owned(),
            created_at: now,
        })
        .map_err(|error| error.to_string())?;
    let _ = app.emit(
        "chat-message",
        ChatMessageEventDto {
            schema_version: SCHEMA_VERSION,
            turn_id: persisted.turn_id,
            message_id: Some(persisted.message_id),
            role: ChatRoleDto::Assistant,
            text: text.to_owned(),
        },
    );
    if tts_enabled {
        let mut tracker = TurnTracker::after(persisted.turn_id.saturating_sub(1));
        app.state::<crate::tts::TtsService>()
            .enqueue(app, tracker.begin_turn(), text);
    }
    Ok(())
}

fn trigger_enabled(settings: &BehaviorSettingsDto, kind: CandidateKind) -> bool {
    match kind {
        CandidateKind::Return => settings.triggers.return_after_enabled,
        CandidateKind::LongSession => settings.triggers.long_session_enabled,
        CandidateKind::CategoryChange => settings.triggers.category_change_enabled,
    }
}

fn candidate_kind_name(kind: CandidateKind) -> &'static str {
    match kind {
        CandidateKind::Return => "return",
        CandidateKind::LongSession => "long_session",
        CandidateKind::CategoryChange => "category_change",
    }
}

fn candidate_cue(candidate: &Candidate) -> String {
    format!(
        "Trigger: {}; broad activity category: {}; duration: {} seconds.",
        candidate_kind_name(candidate.kind()),
        candidate.category().as_str(),
        candidate.duration_seconds()
    )
}

fn frequency_policy(settings: &BehaviorSettingsDto) -> FrequencyPolicy {
    FrequencyPolicy {
        minimum_interval: i64::from(settings.frequency.minimum_interval_minutes) * 60,
        max_per_hour: u64::from(settings.frequency.max_per_hour),
        max_per_day: u64::from(settings.frequency.max_per_day),
    }
}

fn in_quiet_hours(settings: &BehaviorSettingsDto, local: &OffsetDateTime) -> bool {
    let day = local.weekday().number_days_from_monday();
    let minute = u16::from(local.hour()) * 60 + u16::from(local.minute());
    settings.quiet_hours.iter().any(|rule| {
        if !rule.enabled {
            return false;
        }
        let parse = |value: &str| -> Option<u16> {
            let (hour, minute) = value.split_once(':')?;
            Some(hour.parse::<u16>().ok()? * 60 + minute.parse::<u16>().ok()?)
        };
        let (Some(start), Some(end)) = (parse(&rule.start_local_time), parse(&rule.end_local_time))
        else {
            return true;
        };
        if start < end {
            rule.days_of_week.contains(&day) && (start..end).contains(&minute)
        } else {
            let previous = if day == 0 { 6 } else { day - 1 };
            (rule.days_of_week.contains(&day) && minute >= start)
                || (rule.days_of_week.contains(&previous) && minute < end)
        }
    })
}

fn evaluator_approves<R: Runtime>(
    app: &AppHandle<R>,
    settings: &BehaviorSettingsDto,
    cue: &str,
    generated: &str,
) -> bool {
    match (
        settings.evaluator_endpoint.as_deref(),
        settings.evaluator_model.as_deref(),
    ) {
        (None, None) => true,
        (Some(endpoint), Some(model)) => app
            .state::<ChatService>()
            .evaluate_proactive_reply(app, cue, generated, endpoint, model)
            .unwrap_or(false),
        _ => false,
    }
}

fn proactive_thresholds(settings: &BehaviorSettingsDto) -> ProactiveThresholds {
    ProactiveThresholds {
        return_after: i64::from(settings.triggers.return_after_minutes) * 60,
        long_session: i64::from(settings.triggers.long_session_minutes) * 60,
        category_change: i64::from(settings.triggers.category_change_minutes) * 60,
    }
}

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or(-1)
}
