//! Character model manifest contract.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const CHARACTER_MANIFEST_SCHEMA_VERSION: u16 = 2;
pub const CHARACTER_SETTINGS_SCHEMA_VERSION: u16 = 1;
pub const CHARACTER_SETTINGS_CHANGED_EVENT: &str = "character-settings-changed";

/// One motion group of a `Live2D` model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "MotionGroupDto.ts")]
pub struct MotionGroupDto {
    pub name: String,
    pub motion_count: u32,
}

/// Everything a window needs to load and control the active character.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "CharacterManifestDto.ts")]
pub struct CharacterManifestDto {
    pub schema_version: u16,
    pub id: String,
    pub display_name: String,
    pub renderer: CharacterRendererDto,
}

/// Renderer-specific character assets resolved to validated paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export_to = "CharacterRendererDto.ts")]
pub enum CharacterRendererDto {
    Live2d {
        model_path: String,
        default_expression: Option<String>,
        expressions: Vec<String>,
        motion_groups: Vec<MotionGroupDto>,
    },
    StaticImage {
        default_expression: String,
        expressions: Vec<StaticExpressionDto>,
        width: u32,
        height: u32,
    },
}

/// One named full-frame image available to a static renderer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "StaticExpressionDto.ts")]
pub struct StaticExpressionDto {
    pub name: String,
    pub image_path: String,
}

/// Persisted global character selection and behavior settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "CharacterSettingsDto.ts")]
pub struct CharacterSettingsDto {
    pub schema_version: u16,
    pub active_character_id: Option<String>,
    pub expression_idle_timeout_seconds: Option<u32>,
}

impl Default for CharacterSettingsDto {
    fn default() -> Self {
        Self {
            schema_version: CHARACTER_SETTINGS_SCHEMA_VERSION,
            active_character_id: None,
            expression_idle_timeout_seconds: Some(20),
        }
    }
}

/// Event payload emitted after character settings change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "CharacterSettingsChangedEventDto.ts")]
pub struct CharacterSettingsChangedEventDto {
    pub schema_version: u16,
    pub settings: CharacterSettingsDto,
}

#[cfg(test)]
mod tests {
    use super::{
        CHARACTER_MANIFEST_SCHEMA_VERSION, CHARACTER_SETTINGS_SCHEMA_VERSION, CharacterManifestDto,
        CharacterRendererDto, CharacterSettingsDto, StaticExpressionDto,
    };

    #[test]
    fn serializes_static_renderer_contract() {
        let manifest = CharacterManifestDto {
            schema_version: CHARACTER_MANIFEST_SCHEMA_VERSION,
            id: "epsilon-static".into(),
            display_name: "Epsilon Static".into(),
            renderer: CharacterRendererDto::StaticImage {
                default_expression: "neutral".into(),
                expressions: vec![StaticExpressionDto {
                    name: "neutral".into(),
                    image_path: "neutral.png".into(),
                }],
                width: 1024,
                height: 2048,
            },
        };

        assert_eq!(
            serde_json::to_value(manifest).unwrap(),
            serde_json::json!({
                "schema_version": 2,
                "id": "epsilon-static",
                "display_name": "Epsilon Static",
                "renderer": {
                    "kind": "static_image",
                    "default_expression": "neutral",
                    "expressions": [{
                        "name": "neutral",
                        "image_path": "neutral.png",
                    }],
                    "width": 1024,
                    "height": 2048,
                },
            })
        );
    }

    #[test]
    fn character_settings_default_to_twenty_second_idle_timeout() {
        let settings = CharacterSettingsDto::default();

        assert_eq!(settings.schema_version, CHARACTER_SETTINGS_SCHEMA_VERSION);
        assert_eq!(settings.active_character_id, None);
        assert_eq!(settings.expression_idle_timeout_seconds, Some(20));
    }
}
