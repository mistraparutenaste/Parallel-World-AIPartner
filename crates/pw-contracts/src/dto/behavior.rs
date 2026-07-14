//! Context-aware companion behavior and mode contracts.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const BEHAVIOR_SETTINGS_SCHEMA_VERSION: u16 = 1;
pub const BEHAVIOR_SETTINGS_CHANGED_EVENT: &str = "behavior-settings-changed";
pub const ACTIVE_MODE_CHANGED_EVENT: &str = "active-mode-changed";
pub const ACTIVITY_COLLECTION_HEALTH_EVENT: &str = "activity-collection-health";

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "FrequencyPolicyDto.ts")]
pub struct FrequencyPolicyDto {
    pub minimum_interval_minutes: u16,
    pub max_per_hour: u8,
    pub max_per_day: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "TriggerPolicyDto.ts")]
pub struct TriggerPolicyDto {
    pub return_after_minutes: u16,
    pub long_session_minutes: u16,
    pub category_change_minutes: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "ShortcutSettingsDto.ts")]
pub struct ShortcutSettingsDto {
    pub toggle_chat: String,
    pub cycle_mode: String,
    pub pause_proactive: String,
    pub toggle_collection: String,
    pub toggle_focus: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "BehaviorSettingsDto.ts")]
pub struct BehaviorSettingsDto {
    pub schema_version: u16,
    pub consent: ConsentStateDto,
    pub consent_version: u16,
    pub collection_enabled: bool,
    pub retention_days: u16,
    pub exclusions: Vec<ExclusionRuleDto>,
    pub frequency: FrequencyPolicyDto,
    pub triggers: TriggerPolicyDto,
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
            consent: ConsentStateDto::Pending,
            consent_version: 1,
            collection_enabled: false,
            retention_days: 30,
            exclusions: Vec::new(),
            frequency: FrequencyPolicyDto {
                minimum_interval_minutes: 15,
                max_per_hour: 3,
                max_per_day: 16,
            },
            triggers: TriggerPolicyDto {
                return_after_minutes: 10,
                long_session_minutes: 60,
                category_change_minutes: 10,
            },
            evaluator_endpoint: None,
            evaluator_model: None,
            shortcuts: ShortcutSettingsDto {
                toggle_chat: "Ctrl+Alt+Space".to_owned(),
                cycle_mode: "Ctrl+Alt+M".to_owned(),
                pause_proactive: "Ctrl+Alt+P".to_owned(),
                toggle_collection: "Ctrl+Alt+C".to_owned(),
                toggle_focus: "Ctrl+Alt+F".to_owned(),
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
        for (name, profile) in [
            ("normal", &self.profiles.normal),
            ("focus", &self.profiles.focus),
            ("night", &self.profiles.night),
        ] {
            if !profile.volume.is_finite() || !(0.0..=1.0).contains(&profile.volume) {
                return Err(format!("{name} profile volume must be between 0 and 1"));
            }
        }
        Ok(())
    }
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
