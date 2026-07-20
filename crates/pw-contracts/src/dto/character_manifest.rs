//! Character model manifest contract.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const CHARACTER_MANIFEST_SCHEMA_VERSION: u16 = 2;
pub const CHARACTER_SETTINGS_SCHEMA_VERSION: u16 = 3;
pub const CHARACTER_SETUP_SCHEMA_VERSION: u16 = 1;
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

/// Renderer family used by one configured character source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "CharacterRendererKindDto.ts")]
pub enum CharacterRendererKindDto {
    Live2d,
    StaticImage,
}

/// Configuration and activation state for one renderer family.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "CharacterSourceStatusDto.ts")]
pub struct CharacterSourceStatusDto {
    pub kind: CharacterRendererKindDto,
    pub configured: bool,
    pub display_name: Option<String>,
    pub file_name: Option<String>,
    pub import_enabled: bool,
    pub active: bool,
}

/// Combined setup state for all supported character sources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "CharacterSetupDto.ts")]
pub struct CharacterSetupDto {
    pub schema_version: u16,
    pub active_renderer: Option<CharacterRendererKindDto>,
    pub live2d: CharacterSourceStatusDto,
    pub static_image: CharacterSourceStatusDto,
}

/// Persisted global character selection and behavior settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "CharacterSettingsDto.ts")]
pub struct CharacterSettingsDto {
    pub schema_version: u16,
    pub active_character_id: Option<String>,
    #[serde(default)]
    pub live2d_character_id: Option<String>,
    #[serde(default)]
    pub static_image_character_id: Option<String>,
    pub expression_idle_timeout_seconds: Option<u32>,
    #[serde(default = "default_character_size_percent")]
    pub character_size_percent: u16,
}

fn default_character_size_percent() -> u16 {
    100
}

impl Default for CharacterSettingsDto {
    fn default() -> Self {
        Self {
            schema_version: CHARACTER_SETTINGS_SCHEMA_VERSION,
            active_character_id: None,
            live2d_character_id: None,
            static_image_character_id: None,
            expression_idle_timeout_seconds: Some(20),
            character_size_percent: default_character_size_percent(),
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
        CHARACTER_MANIFEST_SCHEMA_VERSION, CHARACTER_SETTINGS_SCHEMA_VERSION,
        CHARACTER_SETUP_SCHEMA_VERSION, CharacterManifestDto, CharacterRendererDto,
        CharacterRendererKindDto, CharacterSettingsDto, CharacterSetupDto,
        CharacterSourceStatusDto, StaticExpressionDto,
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

        assert_eq!(CHARACTER_SETTINGS_SCHEMA_VERSION, 3);
        assert_eq!(
            serde_json::to_value(settings).unwrap(),
            serde_json::json!({
                "schema_version": 3,
                "active_character_id": null,
                "live2d_character_id": null,
                "static_image_character_id": null,
                "expression_idle_timeout_seconds": 20,
                "character_size_percent": 100,
            })
        );
    }

    #[test]
    fn serializes_character_setup_contract() {
        let setup = CharacterSetupDto {
            schema_version: CHARACTER_SETUP_SCHEMA_VERSION,
            active_renderer: Some(CharacterRendererKindDto::StaticImage),
            live2d: CharacterSourceStatusDto {
                kind: CharacterRendererKindDto::Live2d,
                configured: true,
                display_name: Some("Epsilon Live2D".into()),
                file_name: Some("epsilon.model3.json".into()),
                import_enabled: true,
                active: false,
            },
            static_image: CharacterSourceStatusDto {
                kind: CharacterRendererKindDto::StaticImage,
                configured: true,
                display_name: Some("Epsilon Static".into()),
                file_name: Some("epsilon.png".into()),
                import_enabled: false,
                active: true,
            },
        };

        assert_eq!(
            serde_json::to_value(setup).unwrap(),
            serde_json::json!({
                "schema_version": 1,
                "active_renderer": "static_image",
                "live2d": {
                    "kind": "live2d",
                    "configured": true,
                    "display_name": "Epsilon Live2D",
                    "file_name": "epsilon.model3.json",
                    "import_enabled": true,
                    "active": false,
                },
                "static_image": {
                    "kind": "static_image",
                    "configured": true,
                    "display_name": "Epsilon Static",
                    "file_name": "epsilon.png",
                    "import_enabled": false,
                    "active": true,
                },
            })
        );
    }
}
