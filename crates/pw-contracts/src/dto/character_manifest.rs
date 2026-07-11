//! Character model manifest contract.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One motion group of a `Live2D` model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "MotionGroupDto.ts")]
pub struct MotionGroupDto {
    pub name: String,
    pub motion_count: u32,
}

/// Everything a window needs to load and control the active
/// character model. `model_path` is an absolute filesystem path; the
/// frontend converts it to an asset URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "CharacterManifestDto.ts")]
pub struct CharacterManifestDto {
    pub schema_version: u16,
    pub model_path: String,
    pub expressions: Vec<String>,
    pub motion_groups: Vec<MotionGroupDto>,
}

#[cfg(test)]
mod tests {
    use super::{CharacterManifestDto, MotionGroupDto};
    use crate::SCHEMA_VERSION;

    #[test]
    fn serializes_manifest_contract() {
        let value = CharacterManifestDto {
            schema_version: SCHEMA_VERSION,
            model_path: "C:/data/characters/epsilon/Epsilon.model3.json".into(),
            expressions: vec!["Normal".into(), "Smile".into()],
            motion_groups: vec![MotionGroupDto {
                name: "Idle".into(),
                motion_count: 1,
            }],
        };
        let json = serde_json::to_value(value).unwrap();
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["expressions"][1], "Smile");
        assert_eq!(json["motion_groups"][0]["name"], "Idle");
        assert_eq!(json["motion_groups"][0]["motion_count"], 1);
    }
}
