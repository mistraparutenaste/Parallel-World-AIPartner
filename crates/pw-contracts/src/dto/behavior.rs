//! Context-aware companion behavior and mode contracts.

use std::collections::HashSet;

use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;

pub const BEHAVIOR_SETTINGS_SCHEMA_VERSION: u16 = 2;
pub const BEHAVIOR_SETTINGS_CHANGED_EVENT: &str = "behavior-settings-changed";
pub const ACTIVE_MODE_CHANGED_EVENT: &str = "active-mode-changed";
pub const ACTIVITY_COLLECTION_HEALTH_EVENT: &str = "activity-collection-health";
pub const MAX_ACTIVITY_APP_ID_CHARS: usize = 260;
const MAX_MODE_ACTIVATION_RULES: usize = 32;
const MAX_APP_IDS_PER_RULE: usize = 64;
const MAX_QUIET_HOURS_RULES: usize = 32;
const MAX_QUIET_HOURS_RULE_ID_CHARS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "ConsentStateDto.ts")]
pub enum ConsentStateDto {
    Pending,
    Accepted,
    Declined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "CompanionModeDto.ts")]
pub enum CompanionModeDto {
    Normal,
    Focus,
    Night,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "ExclusionRuleDto.ts")]
pub struct ExclusionRuleDto {
    pub app_id: Option<String>,
    pub title_pattern: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "FrequencyPolicyDto.ts")]
pub struct FrequencyPolicyDto {
    pub minimum_interval_minutes: u16,
    pub max_per_hour: u8,
    pub max_per_day: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "TriggerPolicyDto.ts")]
pub struct TriggerPolicyDto {
    pub return_after_enabled: bool,
    pub return_after_minutes: u16,
    pub long_session_enabled: bool,
    pub long_session_minutes: u16,
    pub category_change_enabled: bool,
    pub category_change_minutes: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "QuietHoursRuleDto.ts")]
pub struct QuietHoursRuleDto {
    pub rule_id: String,
    pub enabled: bool,
    pub days_of_week: Vec<u8>,
    pub start_local_time: String,
    pub end_local_time: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "ShortcutSettingsDto.ts")]
pub struct ShortcutSettingsDto {
    pub push_to_talk: String,
    pub toggle_mute: String,
    pub open_control_center: String,
    pub toggle_character: String,
    pub cycle_mode: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[allow(clippy::struct_excessive_bools)]
#[ts(export_to = "ModeProfileDto.ts")]
pub struct ModeProfileDto {
    pub proactive_enabled: bool,
    pub tts_enabled: bool,
    pub character_enabled: bool,
    pub notifications_enabled: bool,
    pub volume: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "ModeProfilesDto.ts")]
pub struct ModeProfilesDto {
    pub normal: ModeProfileDto,
    pub focus: ModeProfileDto,
    pub night: ModeProfileDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "ScheduleActivationRuleDto.ts")]
pub struct ScheduleActivationRuleDto {
    pub enabled: bool,
    pub mode: CompanionModeDto,
    pub days_of_week: Vec<u8>,
    pub start_local_time: String,
    pub end_local_time: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "AppActivationRuleDto.ts")]
pub struct AppActivationRuleDto {
    pub enabled: bool,
    pub mode: CompanionModeDto,
    pub app_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "FullscreenActivationRuleDto.ts")]
pub struct FullscreenActivationRuleDto {
    pub enabled: bool,
    pub mode: CompanionModeDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "ModeActivationRulesDto.ts")]
pub struct ModeActivationRulesDto {
    pub schedules: Vec<ScheduleActivationRuleDto>,
    pub apps: Vec<AppActivationRuleDto>,
    pub fullscreen: FullscreenActivationRuleDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export_to = "BehaviorSettingsDto.ts")]
pub struct BehaviorSettingsDto {
    pub schema_version: u16,
    pub proactive_master_enabled: bool,
    pub consent: ConsentStateDto,
    pub consent_version: u16,
    pub collection_enabled: bool,
    pub retention_days: u16,
    pub exclusions: Vec<ExclusionRuleDto>,
    pub frequency: FrequencyPolicyDto,
    pub triggers: TriggerPolicyDto,
    pub quiet_hours: Vec<QuietHoursRuleDto>,
    #[ts(type = "number | null")]
    pub proactive_snoozed_until: Option<i64>,
    pub evaluator_endpoint: Option<String>,
    pub evaluator_model: Option<String>,
    pub shortcuts: ShortcutSettingsDto,
    pub profiles: ModeProfilesDto,
    pub activation: ModeActivationRulesDto,
    pub manual_mode_override: Option<CompanionModeDto>,
}

impl Default for BehaviorSettingsDto {
    fn default() -> Self {
        let inactive_profile = ModeProfileDto {
            proactive_enabled: false,
            tts_enabled: false,
            character_enabled: false,
            notifications_enabled: false,
            volume: 0.0,
        };
        Self {
            schema_version: BEHAVIOR_SETTINGS_SCHEMA_VERSION,
            proactive_master_enabled: false,
            consent: ConsentStateDto::Pending,
            consent_version: 1,
            collection_enabled: false,
            retention_days: 30,
            exclusions: Vec::new(),
            frequency: FrequencyPolicyDto {
                minimum_interval_minutes: 30,
                max_per_hour: 2,
                max_per_day: 8,
            },
            triggers: TriggerPolicyDto {
                return_after_enabled: true,
                return_after_minutes: 10,
                long_session_enabled: true,
                long_session_minutes: 60,
                category_change_enabled: true,
                category_change_minutes: 10,
            },
            quiet_hours: Vec::new(),
            proactive_snoozed_until: None,
            evaluator_endpoint: None,
            evaluator_model: None,
            shortcuts: ShortcutSettingsDto {
                push_to_talk: "Ctrl+Alt+Space".to_owned(),
                toggle_mute: "Ctrl+Alt+M".to_owned(),
                open_control_center: "Ctrl+Alt+P".to_owned(),
                toggle_character: "Ctrl+Alt+C".to_owned(),
                cycle_mode: "Ctrl+Alt+F".to_owned(),
            },
            profiles: ModeProfilesDto {
                normal: ModeProfileDto {
                    proactive_enabled: true,
                    tts_enabled: true,
                    character_enabled: true,
                    notifications_enabled: false,
                    volume: 1.0,
                },
                focus: inactive_profile.clone(),
                night: inactive_profile,
            },
            activation: ModeActivationRulesDto {
                schedules: Vec::new(),
                apps: Vec::new(),
                fullscreen: FullscreenActivationRuleDto {
                    enabled: false,
                    mode: CompanionModeDto::Focus,
                },
            },
            manual_mode_override: None,
        }
    }
}

#[derive(Deserialize)]
struct BehaviorSettingsWire {
    schema_version: u16,
    proactive_master_enabled: bool,
    consent: ConsentStateDto,
    consent_version: u16,
    collection_enabled: bool,
    retention_days: u16,
    exclusions: Vec<ExclusionRuleDto>,
    frequency: FrequencyPolicyDto,
    triggers: TriggerPolicyDto,
    quiet_hours: Vec<QuietHoursRuleDto>,
    proactive_snoozed_until: Option<i64>,
    evaluator_endpoint: Option<String>,
    evaluator_model: Option<String>,
    shortcuts: ShortcutSettingsDto,
    profiles: ModeProfilesDto,
    activation: ModeActivationRulesDto,
    manual_mode_override: Option<CompanionModeDto>,
}

impl From<BehaviorSettingsWire> for BehaviorSettingsDto {
    fn from(wire: BehaviorSettingsWire) -> Self {
        Self {
            schema_version: wire.schema_version,
            proactive_master_enabled: wire.proactive_master_enabled,
            consent: wire.consent,
            consent_version: wire.consent_version,
            collection_enabled: wire.collection_enabled,
            retention_days: wire.retention_days,
            exclusions: wire.exclusions,
            frequency: wire.frequency,
            triggers: wire.triggers,
            quiet_hours: wire.quiet_hours,
            proactive_snoozed_until: wire.proactive_snoozed_until,
            evaluator_endpoint: wire.evaluator_endpoint,
            evaluator_model: wire.evaluator_model,
            shortcuts: wire.shortcuts,
            profiles: wire.profiles,
            activation: wire.activation,
            manual_mode_override: wire.manual_mode_override,
        }
    }
}

impl<'de> Deserialize<'de> for BehaviorSettingsDto {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut value = serde_json::Value::deserialize(deserializer)?;
        let object = value
            .as_object_mut()
            .ok_or_else(|| serde::de::Error::custom("behavior settings must be an object"))?;
        let schema_version = object
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                serde::de::Error::custom("behavior settings schema_version is missing")
            })?;
        match schema_version {
            1 => migrate_behavior_v1(object).map_err(serde::de::Error::custom)?,
            version if version == u64::from(BEHAVIOR_SETTINGS_SCHEMA_VERSION) => {}
            version => {
                return Err(serde::de::Error::custom(format!(
                    "unsupported behavior settings schema version: {version}"
                )));
            }
        }
        serde_json::from_value::<BehaviorSettingsWire>(value)
            .map(Into::into)
            .map_err(serde::de::Error::custom)
    }
}

fn migrate_behavior_v1(
    object: &mut serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    object.insert(
        "schema_version".to_owned(),
        serde_json::json!(BEHAVIOR_SETTINGS_SCHEMA_VERSION),
    );
    object
        .entry("proactive_master_enabled")
        .or_insert(serde_json::json!(false));
    object
        .entry("quiet_hours")
        .or_insert_with(|| serde_json::json!([]));
    object
        .entry("proactive_snoozed_until")
        .or_insert(serde_json::Value::Null);

    let triggers = object
        .get_mut("triggers")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| "behavior triggers must be an object".to_owned())?;
    triggers
        .entry("return_after_enabled")
        .or_insert(serde_json::json!(true));
    triggers
        .entry("long_session_enabled")
        .or_insert(serde_json::json!(true));
    triggers
        .entry("category_change_enabled")
        .or_insert(serde_json::json!(true));

    if let Some(frequency) = object.get_mut("frequency") {
        migrate_behavior_v1_frequency(frequency);
    }
    Ok(())
}

const FREQUENCY_PRESETS: [FrequencyPolicyDto; 5] = [
    FrequencyPolicyDto {
        minimum_interval_minutes: 180,
        max_per_hour: 1,
        max_per_day: 2,
    },
    FrequencyPolicyDto {
        minimum_interval_minutes: 90,
        max_per_hour: 1,
        max_per_day: 4,
    },
    FrequencyPolicyDto {
        minimum_interval_minutes: 30,
        max_per_hour: 2,
        max_per_day: 8,
    },
    FrequencyPolicyDto {
        minimum_interval_minutes: 15,
        max_per_hour: 3,
        max_per_day: 16,
    },
    FrequencyPolicyDto {
        minimum_interval_minutes: 5,
        max_per_hour: 6,
        max_per_day: 32,
    },
];

fn migrate_behavior_v1_frequency(value: &mut serde_json::Value) {
    let Ok(current) = serde_json::from_value::<FrequencyPolicyDto>(value.clone()) else {
        return;
    };
    let closest = FREQUENCY_PRESETS
        .into_iter()
        .filter(|candidate| {
            candidate.minimum_interval_minutes >= current.minimum_interval_minutes
                && candidate.max_per_hour <= current.max_per_hour
                && candidate.max_per_day <= current.max_per_day
        })
        .min_by_key(|candidate| {
            (
                candidate.minimum_interval_minutes - current.minimum_interval_minutes,
                current.max_per_hour - candidate.max_per_hour,
                current.max_per_day - candidate.max_per_day,
            )
        });
    if let Some(closest) = closest {
        *value = serde_json::to_value(closest)
            .expect("frequency policy with numeric fields always serializes");
    }
}

impl BehaviorSettingsDto {
    /// Validates deterministic transport and persistence invariants.
    ///
    /// # Errors
    ///
    /// Returns a stable message when the schema or a numeric range is invalid.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != BEHAVIOR_SETTINGS_SCHEMA_VERSION {
            return Err(format!(
                "unsupported behavior settings schema version: {}",
                self.schema_version
            ));
        }
        if self.consent_version == 0 {
            return Err("consent_version must be greater than zero".to_owned());
        }
        if self.collection_enabled && self.consent != ConsentStateDto::Accepted {
            return Err("collection requires accepted consent".to_owned());
        }
        if self.retention_days == 0 {
            return Err("retention_days must be greater than zero".to_owned());
        }
        for exclusion in &self.exclusions {
            if exclusion.app_id.is_none() && exclusion.title_pattern.is_none() {
                return Err("activity exclusion must select an app or title".to_owned());
            }
            if exclusion.app_id.as_ref().is_some_and(|value| {
                value.is_empty() || value.len() > 260 || value.contains(char::is_control)
            }) {
                return Err("activity exclusion app_id is invalid".to_owned());
            }
            if exclusion.title_pattern.as_ref().is_some_and(|value| {
                value.is_empty() || value.chars().count() > 128 || value.contains(char::is_control)
            }) {
                return Err("activity exclusion title_pattern is invalid".to_owned());
            }
        }
        if self.frequency.minimum_interval_minutes == 0
            || self.frequency.max_per_hour == 0
            || self.frequency.max_per_day == 0
            || self.frequency.max_per_day < self.frequency.max_per_hour
        {
            return Err(
                "frequency values must be positive and max_per_day must be at least max_per_hour"
                    .to_owned(),
            );
        }
        if self.triggers.return_after_minutes == 0
            || self.triggers.long_session_minutes == 0
            || self.triggers.category_change_minutes == 0
        {
            return Err("trigger intervals must be greater than zero".to_owned());
        }
        validate_quiet_hours(&self.quiet_hours)?;
        if self.proactive_snoozed_until.is_some_and(|value| value < 0) {
            return Err("proactive_snoozed_until must not be negative".to_owned());
        }
        for (name, profile) in [
            ("normal", &self.profiles.normal),
            ("focus", &self.profiles.focus),
            ("night", &self.profiles.night),
        ] {
            if !profile.volume.is_finite() || !(0.0..=1.0).contains(&profile.volume) {
                return Err(format!("{name} profile volume must be between 0 and 1"));
            }
        }
        validate_mode_activation_rules(&self.activation)?;
        Ok(())
    }
}

fn validate_quiet_hours(rules: &[QuietHoursRuleDto]) -> Result<(), String> {
    if rules.len() > MAX_QUIET_HOURS_RULES {
        return Err("quiet hours must contain at most 32 entries".to_owned());
    }
    let mut seen_rule_ids = HashSet::with_capacity(rules.len());
    for rule in rules {
        if rule.rule_id.trim().is_empty()
            || rule.rule_id.chars().count() > MAX_QUIET_HOURS_RULE_ID_CHARS
            || rule.rule_id.contains(char::is_control)
        {
            return Err("quiet hours rule_id is invalid".to_owned());
        }
        if !seen_rule_ids.insert(rule.rule_id.as_str()) {
            return Err("quiet hours rule_id must be unique".to_owned());
        }
        if rule.days_of_week.is_empty() {
            return Err("quiet hours days must not be empty".to_owned());
        }
        let mut seen_days = [false; 7];
        for &day in &rule.days_of_week {
            let Some(seen) = seen_days.get_mut(usize::from(day)) else {
                return Err("quiet hours days must be between 0 and 6".to_owned());
            };
            if *seen {
                return Err("quiet hours days must be unique".to_owned());
            }
            *seen = true;
        }
        let start = parse_local_time(&rule.start_local_time)
            .ok_or_else(|| "quiet hours start time must use HH:MM".to_owned())?;
        let end = parse_local_time(&rule.end_local_time)
            .ok_or_else(|| "quiet hours end time must use HH:MM".to_owned())?;
        if start == end {
            return Err("quiet hours start and end times must differ".to_owned());
        }
    }
    Ok(())
}

fn validate_mode_activation_rules(rules: &ModeActivationRulesDto) -> Result<(), String> {
    if rules.schedules.len() > MAX_MODE_ACTIVATION_RULES {
        return Err("schedule activation rules must contain at most 32 entries".to_owned());
    }
    if rules.apps.len() > MAX_MODE_ACTIVATION_RULES {
        return Err("app activation rules must contain at most 32 entries".to_owned());
    }

    for rule in &rules.schedules {
        if rule.days_of_week.is_empty() {
            return Err("schedule activation days must not be empty".to_owned());
        }
        let mut seen_days = [false; 7];
        for &day in &rule.days_of_week {
            let Some(seen) = seen_days.get_mut(usize::from(day)) else {
                return Err("schedule activation days must be between 0 and 6".to_owned());
            };
            if *seen {
                return Err("schedule activation days must be unique".to_owned());
            }
            *seen = true;
        }

        let start = parse_local_time(&rule.start_local_time)
            .ok_or_else(|| "schedule activation start time must use HH:MM".to_owned())?;
        let end = parse_local_time(&rule.end_local_time)
            .ok_or_else(|| "schedule activation end time must use HH:MM".to_owned())?;
        if start == end {
            return Err("schedule activation start and end times must differ".to_owned());
        }
    }

    for rule in &rules.apps {
        if rule.app_ids.is_empty() || rule.app_ids.len() > MAX_APP_IDS_PER_RULE {
            return Err("app activation rule must contain between 1 and 64 app ids".to_owned());
        }
        let mut normalized_ids = HashSet::with_capacity(rule.app_ids.len());
        for app_id in &rule.app_ids {
            if app_id.trim().is_empty() || app_id.contains(char::is_control) {
                return Err("app activation app_id is invalid".to_owned());
            }
            let normalized = normalize_activity_app_id(app_id)
                .ok_or_else(|| "app activation app_id is invalid".to_owned())?;
            if !normalized_ids.insert(normalized) {
                return Err("app activation app_ids must be unique".to_owned());
            }
        }
    }
    Ok(())
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

/// Produces the bounded Unicode-lowercase form used to compare foreground app ids.
#[must_use]
pub fn normalize_activity_app_id(value: &str) -> Option<String> {
    let mut chars = value.chars();
    let mut lowercase = String::with_capacity(MAX_ACTIVITY_APP_ID_CHARS.saturating_mul(4));
    for _ in 0..MAX_ACTIVITY_APP_ID_CHARS {
        let Some(character) = chars.next() else {
            return Some(lowercase);
        };
        lowercase.extend(character.to_lowercase());
    }
    chars.next().is_none().then_some(lowercase)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "ActiveModeSourceDto.ts")]
pub enum ActiveModeSourceDto {
    Default,
    Manual,
    Schedule,
    App,
    Fullscreen,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "ActiveModeDto.ts")]
pub struct ActiveModeDto {
    pub schema_version: u16,
    pub mode: CompanionModeDto,
    pub source: ActiveModeSourceDto,
    pub manual_override: Option<CompanionModeDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "BehaviorSettingsChangedEventDto.ts")]
pub struct BehaviorSettingsChangedEventDto {
    pub schema_version: u16,
    pub settings: BehaviorSettingsDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "ActiveModeChangedEventDto.ts")]
pub struct ActiveModeChangedEventDto {
    pub schema_version: u16,
    pub active_mode: ActiveModeDto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "ActivityCollectionHealthStatusDto.ts")]
pub enum ActivityCollectionHealthStatusDto {
    Disabled,
    Healthy,
    Degraded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "ActivityCollectionHealthEventDto.ts")]
pub struct ActivityCollectionHealthEventDto {
    pub schema_version: u16,
    pub status: ActivityCollectionHealthStatusDto,
    #[ts(type = "number | null")]
    pub last_activity_at: Option<i64>,
    pub message: Option<String>,
}
