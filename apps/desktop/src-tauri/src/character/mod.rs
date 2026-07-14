//! Character model discovery and manifest parsing.

mod catalog;
mod manifest;

pub use catalog::{
    CharacterCapabilities, CharacterCatalog, CharacterProfileError, ResolvedCharacter,
    ResolvedRenderer, ResolvedStaticExpression,
};
pub use manifest::{CharacterManifest, ManifestError, find_first_model3, parse_model3_json};
