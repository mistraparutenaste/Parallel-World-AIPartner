//! Deterministic resolution of context-aware companion behavior profiles.

use pw_contracts::{
    ActiveModeDto, ActiveModeSourceDto, AppActivationRuleDto, BEHAVIOR_SETTINGS_SCHEMA_VERSION,
    BehaviorSettingsDto, CompanionModeDto, ModeProfileDto, ScheduleActivationRuleDto,
    normalize_activity_app_id,
};
use thiserror::Error;

/// Local context supplied by platform adapters to the pure mode resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeResolutionInput {
    /// Monday is zero and Sunday is six.
    pub local_weekday: u8,
    /// Minutes since local midnight in the inclusive range `0..=1439`.
    pub local_minutes: u16,
    /// Foreground application id, if one could be resolved.
    pub foreground_app_id: Option<String>,
    /// Whether the foreground window is fullscreen, or `None` when unknown.
    pub fullscreen: Option<bool>,
}

/// The selected transport-facing mode and the exact associated behavior profile.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedMode {
    pub active_mode: ActiveModeDto,
    pub profile: ModeProfileDto,
}

/// Stable validation failures for local resolver input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ModeResolutionError {
    #[error("local weekday must be between 0 and 6")]
    InvalidWeekday,
    #[error("local minutes must be between 0 and 1439")]
    InvalidLocalMinutes,
}

/// Resolves the active profile with precedence manual, fullscreen, app, schedule, default.
///
/// # Errors
///
/// Returns a stable error when the supplied local weekday or minute is outside its contract.
pub fn resolve_mode(
    settings: &BehaviorSettingsDto,
    input: &ModeResolutionInput,
) -> Result<ResolvedMode, ModeResolutionError> {
    validate_input(input)?;

    let selection = settings
        .manual_mode_override
        .map(|mode| (mode, ActiveModeSourceDto::Manual))
        .or_else(|| resolve_fullscreen(settings, input))
        .or_else(|| resolve_app(settings, input))
        .or_else(|| resolve_schedule(settings, input))
        .unwrap_or((CompanionModeDto::Normal, ActiveModeSourceDto::Default));

    Ok(resolved(settings, selection.0, selection.1))
}

fn validate_input(input: &ModeResolutionInput) -> Result<(), ModeResolutionError> {
    if input.local_weekday > 6 {
        return Err(ModeResolutionError::InvalidWeekday);
    }
    if input.local_minutes > 1_439 {
        return Err(ModeResolutionError::InvalidLocalMinutes);
    }
    Ok(())
}

fn resolve_fullscreen(
    settings: &BehaviorSettingsDto,
    input: &ModeResolutionInput,
) -> Option<(CompanionModeDto, ActiveModeSourceDto)> {
    let rule = &settings.activation.fullscreen;
    (rule.enabled && input.fullscreen == Some(true))
        .then_some((rule.mode, ActiveModeSourceDto::Fullscreen))
}

fn resolve_app(
    settings: &BehaviorSettingsDto,
    input: &ModeResolutionInput,
) -> Option<(CompanionModeDto, ActiveModeSourceDto)> {
    let foreground = normalize_activity_app_id(input.foreground_app_id.as_deref()?)?;
    quietest_matching_mode(&settings.activation.apps, |rule| {
        rule.enabled
            && rule.app_ids.iter().any(|app_id| {
                normalize_activity_app_id(app_id).is_some_and(|app_id| app_id == foreground)
            })
    })
    .map(|mode| (mode, ActiveModeSourceDto::App))
}

fn quietest_matching_mode<T>(
    rules: &[T],
    mut matches: impl FnMut(&T) -> bool,
) -> Option<CompanionModeDto>
where
    T: ModeRule,
{
    rules
        .iter()
        .filter(|rule| matches(rule))
        .map(ModeRule::mode)
        .max_by_key(|mode| mode_severity(*mode))
}

trait ModeRule {
    fn mode(&self) -> CompanionModeDto;
}

impl ModeRule for AppActivationRuleDto {
    fn mode(&self) -> CompanionModeDto {
        self.mode
    }
}

impl ModeRule for ScheduleActivationRuleDto {
    fn mode(&self) -> CompanionModeDto {
        self.mode
    }
}

const fn mode_severity(mode: CompanionModeDto) -> u8 {
    match mode {
        CompanionModeDto::Normal => 0,
        CompanionModeDto::Focus => 1,
        CompanionModeDto::Night => 2,
    }
}

fn resolve_schedule(
    settings: &BehaviorSettingsDto,
    input: &ModeResolutionInput,
) -> Option<(CompanionModeDto, ActiveModeSourceDto)> {
    quietest_matching_mode(&settings.activation.schedules, |rule| {
        rule.enabled && schedule_is_active(rule, input.local_weekday, input.local_minutes)
    })
    .map(|mode| (mode, ActiveModeSourceDto::Schedule))
}

fn schedule_is_active(rule: &ScheduleActivationRuleDto, weekday: u8, minutes: u16) -> bool {
    let Some(start) = parse_local_time(&rule.start_local_time) else {
        return false;
    };
    let Some(end) = parse_local_time(&rule.end_local_time) else {
        return false;
    };
    if start < end {
        return rule.days_of_week.contains(&weekday) && (start..end).contains(&minutes);
    }
    if start == end {
        return false;
    }

    let previous_weekday = if weekday == 0 { 6 } else { weekday - 1 };
    (rule.days_of_week.contains(&weekday) && minutes >= start)
        || (rule.days_of_week.contains(&previous_weekday) && minutes < end)
}

fn parse_local_time(value: &str) -> Option<u16> {
    let bytes = value.as_bytes();
    if bytes.len() != 5
        || bytes[2] != b':'
        || !bytes[0].is_ascii_digit()
        || !bytes[1].is_ascii_digit()
        || !bytes[3].is_ascii_digit()
        || !bytes[4].is_ascii_digit()
    {
        return None;
    }
    let hours = u16::from(bytes[0] - b'0') * 10 + u16::from(bytes[1] - b'0');
    let minutes = u16::from(bytes[3] - b'0') * 10 + u16::from(bytes[4] - b'0');
    (hours < 24 && minutes < 60).then_some(hours * 60 + minutes)
}

fn resolved(
    settings: &BehaviorSettingsDto,
    mode: CompanionModeDto,
    source: ActiveModeSourceDto,
) -> ResolvedMode {
    let profile = match mode {
        CompanionModeDto::Normal => &settings.profiles.normal,
        CompanionModeDto::Focus => &settings.profiles.focus,
        CompanionModeDto::Night => &settings.profiles.night,
    };
    ResolvedMode {
        active_mode: ActiveModeDto {
            schema_version: BEHAVIOR_SETTINGS_SCHEMA_VERSION,
            mode,
            source,
            manual_override: settings.manual_mode_override,
        },
        profile: profile.clone(),
    }
}
