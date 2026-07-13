use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const CHARACTER_PRESENTATION_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "CharacterPresentationSettingsDto.ts")]
pub struct CharacterPresentationSettingsDto {
    pub schema_version: u16,
    pub revision: u32,
    pub model_id: String,
    pub expression_id: String,
    pub motion_group: String,
    pub motion_index: u32,
    pub click_through: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_versioned_character_presentation_contract() {
        let dto = CharacterPresentationSettingsDto {
            schema_version: CHARACTER_PRESENTATION_SCHEMA_VERSION,
            revision: 7,
            model_id: "epsilon-free".into(),
            expression_id: "Smile".into(),
            motion_group: "Tap".into(),
            motion_index: 0,
            click_through: false,
        };
        let json = serde_json::to_value(dto).unwrap();
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["revision"], 7);
        assert_eq!(json["motion_index"], 0);
        assert_eq!(json["click_through"], false);
    }

    #[test]
    fn rejects_non_boolean_click_through_state() {
        let json = r#"{"schema_version":1,"revision":0,"model_id":"mark","expression_id":"","motion_group":"Idle","motion_index":0,"click_through":"enabled"}"#;
        assert!(serde_json::from_str::<CharacterPresentationSettingsDto>(json).is_err());
    }
}
