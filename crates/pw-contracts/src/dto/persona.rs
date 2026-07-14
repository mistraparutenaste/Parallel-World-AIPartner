//! Per-character persona profile contracts.

use std::collections::BTreeMap;
use std::fmt;

use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;

pub const PERSONA_SETTINGS_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "PersonaProfileDto.ts")]
pub struct PersonaProfileDto {
    pub character_id: String,
    pub name: String,
    pub first_person_pronoun: String,
    pub user_name: String,
    pub user_address: String,
    pub relationship: String,
    pub speaking_style: String,
    pub interests: Vec<String>,
    pub dislikes: Vec<String>,
    pub values: Vec<String>,
    pub background: String,
    pub boundaries: Vec<String>,
    pub free_text: String,
    pub preset: Option<String>,
    pub initiative: u8,
    pub closeness: u8,
    pub humor: u8,
    pub response_length: u8,
    pub emotional_expression: u8,
    pub reaction_interval: u8,
}

impl PersonaProfileDto {
    #[must_use]
    pub fn for_character(character_id: impl Into<String>) -> Self {
        Self {
            character_id: character_id.into(),
            name: String::new(),
            first_person_pronoun: String::new(),
            user_name: String::new(),
            user_address: String::new(),
            relationship: String::new(),
            speaking_style: String::new(),
            interests: Vec::new(),
            dislikes: Vec::new(),
            values: Vec::new(),
            background: String::new(),
            boundaries: Vec::new(),
            free_text: String::new(),
            preset: None,
            initiative: 50,
            closeness: 50,
            humor: 50,
            response_length: 50,
            emotional_expression: 50,
            reaction_interval: 50,
        }
    }

    /// Validates the stable character identity and all six sliders.
    ///
    /// # Errors
    ///
    /// Returns an error when the identity is empty or any slider exceeds 100.
    pub fn validate(&self) -> Result<(), String> {
        if self.character_id.trim().is_empty() {
            return Err("character_id must not be empty".to_owned());
        }
        for (name, value) in [
            ("initiative", self.initiative),
            ("closeness", self.closeness),
            ("humor", self.humor),
            ("response_length", self.response_length),
            ("emotional_expression", self.emotional_expression),
            ("reaction_interval", self.reaction_interval),
        ] {
            if value > 100 {
                return Err(format!("{name} must be between 0 and 100"));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export_to = "PersonaSettingsDto.ts")]
pub struct PersonaSettingsDto {
    pub schema_version: u16,
    pub personas: BTreeMap<String, PersonaProfileDto>,
}

impl<'de> Deserialize<'de> for PersonaSettingsDto {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            schema_version: u16,
            #[serde(deserialize_with = "deserialize_personas")]
            personas: BTreeMap<String, PersonaProfileDto>,
        }

        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            schema_version: wire.schema_version,
            personas: wire.personas,
        })
    }
}

fn deserialize_personas<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, PersonaProfileDto>, D::Error>
where
    D: Deserializer<'de>,
{
    struct PersonasVisitor;

    impl<'de> Visitor<'de> for PersonasVisitor {
        type Value = BTreeMap<String, PersonaProfileDto>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a map with unique character ids")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut personas = BTreeMap::new();
            while let Some((key, profile)) = map.next_entry::<String, PersonaProfileDto>()? {
                if personas.insert(key.clone(), profile).is_some() {
                    return Err(serde::de::Error::custom(format!(
                        "duplicate persona identity: {key}"
                    )));
                }
            }
            Ok(personas)
        }
    }

    deserializer.deserialize_map(PersonasVisitor)
}

impl Default for PersonaSettingsDto {
    fn default() -> Self {
        Self {
            schema_version: PERSONA_SETTINGS_SCHEMA_VERSION,
            personas: BTreeMap::new(),
        }
    }
}

impl PersonaSettingsDto {
    /// Validates schema, profile ranges, and exact map-key identity matches.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsupported schema or mismatched identity.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != PERSONA_SETTINGS_SCHEMA_VERSION {
            return Err(format!(
                "unsupported persona settings schema version: {}",
                self.schema_version
            ));
        }
        for (key, profile) in &self.personas {
            profile.validate()?;
            if key != &profile.character_id {
                return Err(format!(
                    "persona key {key:?} does not match character_id {:?}",
                    profile.character_id
                ));
            }
        }
        Ok(())
    }
}
