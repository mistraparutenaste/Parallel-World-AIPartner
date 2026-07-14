//! Character model discovery and manifest parsing.

mod catalog;
mod manifest;
mod settings;

pub use catalog::{
    CharacterCapabilities, CharacterCatalog, CharacterProfileError, LEGACY_CHARACTER_ID,
    ResolvedCharacter, ResolvedRenderer, ResolvedStaticExpression,
};
pub use manifest::{CharacterManifest, ManifestError, find_first_model3, parse_model3_json};
pub use settings::{
    load_character_settings, save_character_settings, validate_idle_timeout,
    with_expression_idle_timeout,
};
