//! Character model discovery and manifest parsing.

mod manifest;

pub use manifest::{CharacterManifest, ManifestError, find_first_model3, parse_model3_json};
