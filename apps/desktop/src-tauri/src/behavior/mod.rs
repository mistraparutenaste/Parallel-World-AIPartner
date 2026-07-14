//! Context-aware companion settings persistence.

mod atomic_json;
mod personas;
mod settings;

pub use personas::{load_persona, migrate_legacy_character_prompt, save_persona_settings};
pub use settings::{load_behavior_settings, save_behavior_settings};
