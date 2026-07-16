//! User-wide dark-expression safety contracts.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const DARK_EXPRESSION_SAFETY_SCHEMA_VERSION: u16 = 1;
pub const DARK_EXPRESSION_SAFETY_CHANGED_EVENT: &str = "dark-expression-safety-changed";
pub const SAFEWORD_TRIGGERED_EVENT: &str = "safeword-triggered";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "DarkExpressionSafetySettingsDto.ts")]
pub struct DarkExpressionSafetySettingsDto {
    pub schema_version: u16,
    pub safe_word: Option<String>,
    pub dark_expression_paused: bool,
}

impl Default for DarkExpressionSafetySettingsDto {
    fn default() -> Self {
        Self {
            schema_version: DARK_EXPRESSION_SAFETY_SCHEMA_VERSION,
            safe_word: None,
            dark_expression_paused: false,
        }
    }
}

impl DarkExpressionSafetySettingsDto {
    /// Validates the transport shape without logging or exposing the secret phrase.
    ///
    /// # Errors
    ///
    /// Returns a stable message when the schema or bounded phrase is invalid.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != DARK_EXPRESSION_SAFETY_SCHEMA_VERSION {
            return Err(format!(
                "unsupported dark expression safety schema version: {}",
                self.schema_version
            ));
        }
        if self.safe_word.as_ref().is_some_and(|value| {
            value.trim().is_empty()
                || value.chars().count() > 128
                || value.contains(char::is_control)
        }) {
            return Err("safe_word is invalid".to_owned());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "DarkExpressionSafetyChangedEventDto.ts")]
pub struct DarkExpressionSafetyChangedEventDto {
    pub schema_version: u16,
    pub settings: DarkExpressionSafetySettingsDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "SafewordTriggeredEventDto.ts")]
pub struct SafewordTriggeredEventDto {
    pub schema_version: u16,
    pub pause_persisted: bool,
}
