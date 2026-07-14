//! Atomic persistence for `config/behavior.json`.

use std::fs;

use pw_contracts::BehaviorSettingsDto;
use pw_platform::paths::AppDataLayout;

use super::atomic_json::write_atomic_json;

const FILE_NAME: &str = "behavior.json";

/// Loads behavior settings, returning privacy-safe defaults for any invalid input.
#[must_use]
pub fn load_behavior_settings(layout: &AppDataLayout) -> BehaviorSettingsDto {
    let path = layout.config.join(FILE_NAME);
    let Ok(raw) = fs::read_to_string(&path) else {
        return BehaviorSettingsDto::default();
    };
    match serde_json::from_str::<BehaviorSettingsDto>(&raw) {
        Ok(settings) if settings.validate().is_ok() => settings,
        Ok(_) | Err(_) => BehaviorSettingsDto::default(),
    }
}

/// Validates and atomically replaces `config/behavior.json`.
///
/// # Errors
///
/// Returns a validation, serialization, or filesystem error.
pub fn save_behavior_settings(
    layout: &AppDataLayout,
    settings: &BehaviorSettingsDto,
) -> Result<(), String> {
    settings.validate()?;
    write_atomic_json(&layout.config, FILE_NAME, settings)
}
