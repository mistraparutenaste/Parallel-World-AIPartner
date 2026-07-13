use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::SCHEMA_VERSION;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "ThemePreferenceDto.ts")]
pub enum ThemePreferenceDto {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "ChatPlacementDto.ts")]
pub enum ChatPlacementDto {
    #[default]
    Docked,
    Popped,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "UiPreferencesDto.ts")]
pub struct UiPreferencesDto {
    pub schema_version: u16,
    pub theme: ThemePreferenceDto,
    pub chat_placement: ChatPlacementDto,
}

impl Default for UiPreferencesDto {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            theme: ThemePreferenceDto::System,
            chat_placement: ChatPlacementDto::Docked,
        }
    }
}
