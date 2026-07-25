//! Atomic persistence for `config/behavior.json`.

use std::fs;

use pw_contracts::BehaviorSettingsDto;
use pw_platform::config_io::{JsonFormat, write_atomic_json};
use pw_platform::paths::AppDataLayout;
use thiserror::Error;

const FILE_NAME: &str = "behavior.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BehaviorSettingsLoadError {
    #[error("behavior settings could not be read")]
    Io,
    #[error("behavior settings are invalid")]
    Invalid,
}

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

/// Loads behavior settings while distinguishing invalid/unreadable input from
/// an absent file. Error text never contains file contents or paths.
///
/// # Errors
/// Returns a stable error for unreadable, malformed, wrong-schema, or invalid settings.
pub fn load_behavior_settings_checked(
    layout: &AppDataLayout,
) -> Result<BehaviorSettingsDto, BehaviorSettingsLoadError> {
    let path = layout.config.join(FILE_NAME);
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(BehaviorSettingsDto::default());
        }
        Err(_) => return Err(BehaviorSettingsLoadError::Io),
    };
    let settings = serde_json::from_str::<BehaviorSettingsDto>(&raw)
        .map_err(|_| BehaviorSettingsLoadError::Invalid)?;
    settings
        .validate()
        .map_err(|_| BehaviorSettingsLoadError::Invalid)?;
    Ok(settings)
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
    write_atomic_json(
        &layout.config,
        FILE_NAME,
        settings,
        JsonFormat::PrettyWithTrailingNewline,
    )
    .map_err(|error| error.to_string())
}
