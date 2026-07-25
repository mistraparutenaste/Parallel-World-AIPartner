//! Atomic persistence for `config/behavior.json`.

use pw_contracts::BehaviorSettingsDto;
use pw_platform::config_io::{JsonFormat, ReadJsonError, read_json, write_atomic_json};
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
    load_behavior_settings_checked(layout).unwrap_or_default()
}

/// Loads behavior settings while distinguishing invalid/unreadable input from
/// an absent file. Error text never contains file contents or paths.
///
/// # Errors
/// Returns a stable error for unreadable, malformed, wrong-schema, or invalid settings.
pub fn load_behavior_settings_checked(
    layout: &AppDataLayout,
) -> Result<BehaviorSettingsDto, BehaviorSettingsLoadError> {
    let settings = match read_json::<BehaviorSettingsDto>(&layout.config.join(FILE_NAME)) {
        Ok(None) => return Ok(BehaviorSettingsDto::default()),
        Ok(Some(settings)) => settings,
        Err(ReadJsonError::Io(_)) => return Err(BehaviorSettingsLoadError::Io),
        Err(ReadJsonError::Parse(_)) => return Err(BehaviorSettingsLoadError::Invalid),
    };
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
