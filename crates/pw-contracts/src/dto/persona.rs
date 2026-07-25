//! Per-character persona profile contracts.

use std::collections::BTreeMap;
use std::fmt;

use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;

pub const PERSONA_SETTINGS_SCHEMA_VERSION: u16 = 3;
pub const DARK_EXPRESSION_ACKNOWLEDGEMENT_VERSION: u16 = 2;

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
    /// Verbatim example lines the character would say; rendered into the
    /// prompt as few-shot tone samples. Missing in files written before v3.1.
    #[serde(default)]
    pub example_utterances: Vec<String>,
    pub interests: Vec<String>,
    pub dislikes: Vec<String>,
    pub values: Vec<String>,
    pub background: String,
    pub boundaries: Vec<String>,
    pub free_text: String,
    pub initiative: u8,
    pub closeness: u8,
    pub humor: u8,
    pub response_length: u8,
    pub emotional_expression: u8,
    pub reaction_interval: u8,
    pub machiavellianism: u8,
    pub narcissism: u8,
    pub psychopathy: u8,
    pub sadism: u8,
    pub allow_intense_dark_expression: bool,
    pub dark_expression_acknowledgement_version: Option<u16>,
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
            example_utterances: Vec::new(),
            interests: Vec::new(),
            dislikes: Vec::new(),
            values: Vec::new(),
            background: String::new(),
            boundaries: Vec::new(),
            free_text: String::new(),
            initiative: 50,
            closeness: 50,
            humor: 50,
            response_length: 50,
            emotional_expression: 50,
            reaction_interval: 50,
            machiavellianism: 50,
            narcissism: 50,
            psychopathy: 50,
            sadism: 50,
            allow_intense_dark_expression: false,
            dark_expression_acknowledgement_version: None,
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
        if self.example_utterances.len() > 10 {
            return Err("example_utterances must contain at most 10 lines".to_owned());
        }
        if self
            .example_utterances
            .iter()
            .any(|utterance| utterance.chars().count() > 200)
        {
            return Err("each example utterance must be 200 characters or fewer".to_owned());
        }
        for (name, value) in [
            ("initiative", self.initiative),
            ("closeness", self.closeness),
            ("humor", self.humor),
            ("response_length", self.response_length),
            ("emotional_expression", self.emotional_expression),
            ("reaction_interval", self.reaction_interval),
            ("machiavellianism", self.machiavellianism),
            ("narcissism", self.narcissism),
            ("psychopathy", self.psychopathy),
            ("sadism", self.sadism),
        ] {
            if value > 100 {
                return Err(format!("{name} must be between 0 and 100"));
            }
        }
        if self.allow_intense_dark_expression
            && self.dark_expression_acknowledgement_version
                != Some(DARK_EXPRESSION_ACKNOWLEDGEMENT_VERSION)
        {
            return Err("intense dark expression requires current acknowledgement".to_owned());
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
            #[serde(deserialize_with = "deserialize_persona_values")]
            personas: BTreeMap<String, serde_json::Value>,
        }

        let wire = Wire::deserialize(deserializer)?;
        if !matches!(wire.schema_version, 1 | 2 | PERSONA_SETTINGS_SCHEMA_VERSION) {
            return Err(serde::de::Error::custom(format!(
                "unsupported persona settings schema version: {}",
                wire.schema_version
            )));
        }
        let mut personas = BTreeMap::new();
        for (key, mut value) in wire.personas {
            if wire.schema_version == 1 {
                let object = value.as_object_mut().ok_or_else(|| {
                    serde::de::Error::custom(format!("persona {key:?} must be an object"))
                })?;
                for (name, default) in [
                    ("machiavellianism", serde_json::json!(50)),
                    ("narcissism", serde_json::json!(50)),
                    ("psychopathy", serde_json::json!(50)),
                    ("allow_intense_dark_expression", serde_json::json!(false)),
                    (
                        "dark_expression_acknowledgement_version",
                        serde_json::Value::Null,
                    ),
                ] {
                    object.entry(name).or_insert(default);
                }
            }
            if wire.schema_version < PERSONA_SETTINGS_SCHEMA_VERSION {
                let object = value.as_object_mut().ok_or_else(|| {
                    serde::de::Error::custom(format!("persona {key:?} must be an object"))
                })?;
                object
                    .entry("sadism")
                    .or_insert_with(|| serde_json::json!(50));
                object.insert(
                    "allow_intense_dark_expression".to_owned(),
                    serde_json::json!(false),
                );
                object.insert(
                    "dark_expression_acknowledgement_version".to_owned(),
                    serde_json::Value::Null,
                );
            }
            let profile = serde_json::from_value::<PersonaProfileDto>(value)
                .map_err(serde::de::Error::custom)?;
            personas.insert(key, profile);
        }
        Ok(Self {
            schema_version: PERSONA_SETTINGS_SCHEMA_VERSION,
            personas,
        })
    }
}

fn deserialize_persona_values<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, serde_json::Value>, D::Error>
where
    D: Deserializer<'de>,
{
    struct PersonasVisitor;

    impl<'de> Visitor<'de> for PersonasVisitor {
        type Value = BTreeMap<String, serde_json::Value>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a map with unique character ids")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut personas = BTreeMap::new();
            while let Some((key, profile)) = map.next_entry::<String, serde_json::Value>()? {
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
