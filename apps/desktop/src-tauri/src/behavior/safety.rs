//! Atomic user-wide dark-expression safety persistence and matching.

use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};

use pw_contracts::DarkExpressionSafetySettingsDto;
use pw_platform::config_io::{JsonFormat, write_atomic_json};
use pw_platform::paths::AppDataLayout;
use thiserror::Error;
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

const FILE_NAME: &str = "dark-expression-safety.json";

/// Process-local latch that makes a triggered stop effective before persistence.
pub struct DarkExpressionSafetyState {
    paused: AtomicBool,
}

impl Default for DarkExpressionSafetyState {
    fn default() -> Self {
        Self {
            paused: AtomicBool::new(false),
        }
    }
}

impl DarkExpressionSafetyState {
    #[must_use]
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }

    pub fn pause(&self) {
        self.paused.store(true, Ordering::Release);
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Release);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DarkExpressionSafetyLoadError {
    #[error("dark expression safety settings could not be read")]
    Io,
    #[error("dark expression safety settings are invalid")]
    Invalid,
}

/// Loads safety settings, pausing dark expression on any existing invalid input.
#[must_use]
pub fn load_dark_expression_safety(layout: &AppDataLayout) -> DarkExpressionSafetySettingsDto {
    match load_dark_expression_safety_checked(layout) {
        Ok(settings) => settings,
        Err(_) => DarkExpressionSafetySettingsDto {
            dark_expression_paused: true,
            ..DarkExpressionSafetySettingsDto::default()
        },
    }
}

/// Loads safety settings without replacing or rewriting malformed bytes.
///
/// # Errors
///
/// Returns a stable read or validation error that never contains the safe word.
pub fn load_dark_expression_safety_checked(
    layout: &AppDataLayout,
) -> Result<DarkExpressionSafetySettingsDto, DarkExpressionSafetyLoadError> {
    let path = layout.config.join(FILE_NAME);
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DarkExpressionSafetySettingsDto::default());
        }
        Err(_) => return Err(DarkExpressionSafetyLoadError::Io),
    };
    let settings = serde_json::from_str::<DarkExpressionSafetySettingsDto>(&raw)
        .map_err(|_| DarkExpressionSafetyLoadError::Invalid)?;
    settings
        .validate()
        .map_err(|_| DarkExpressionSafetyLoadError::Invalid)?;
    Ok(settings)
}

/// Validates and atomically replaces the user-wide safety file.
///
/// # Errors
///
/// Returns a validation, serialization, or filesystem error without the phrase.
pub fn save_dark_expression_safety(
    layout: &AppDataLayout,
    settings: &DarkExpressionSafetySettingsDto,
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

/// Trims an entered phrase while preserving its user-visible spelling.
///
/// # Errors
///
/// Returns a stable validation error for control characters or excessive length.
pub fn sanitize_safe_word(value: Option<String>) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim_matches(char::is_whitespace);
    if trimmed.is_empty() || normalize_safe_word(trimmed).is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > 128 || trimmed.contains(char::is_control) {
        return Err("safe_word is invalid".to_owned());
    }
    Ok(Some(trimmed.to_owned()))
}

/// Matches a complete user utterance after deterministic Unicode normalization.
#[must_use]
pub fn safe_word_matches(safe_word: Option<&str>, user_input: &str) -> bool {
    let Some(safe_word) = safe_word else {
        return false;
    };
    let expected = normalize_safe_word(safe_word);
    !expected.is_empty() && expected == normalize_safe_word(user_input)
}

fn normalize_safe_word(value: &str) -> String {
    let folded = value.nfkc().case_fold().collect::<String>();
    let trimmed = folded.trim_matches(char::is_whitespace);
    let without_trailing_punctuation =
        trimmed.trim_end_matches(['。', '．', '.', '、', '，', ',', '！', '!', '？', '?']);
    without_trailing_punctuation
        .trim_end_matches(char::is_whitespace)
        .to_owned()
}
