//! Character model discovery and manifest parsing.

mod catalog;
mod manifest;
mod settings;
mod setup;

pub(crate) use catalog::validate_profile_manifest;
pub use catalog::{
    CharacterCapabilities, CharacterCatalog, CharacterProfileError, LEGACY_CHARACTER_ID,
    ResolvedCharacter, ResolvedRenderer, ResolvedStaticExpression,
};
pub use manifest::{CharacterManifest, ManifestError, find_first_model3, parse_model3_json};
pub use settings::{
    load_character_settings, save_character_settings, validate_character_size,
    validate_idle_timeout, with_character_size, with_expression_idle_timeout,
};
pub(crate) use setup::{discover_setup, import_character_source, select_active_renderer};
