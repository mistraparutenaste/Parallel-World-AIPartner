//! Atomic persistence for `config/personas.json`.

use std::fs;

use pw_contracts::{LlmSettingsDto, PersonaProfileDto, PersonaSettingsDto};
use pw_platform::paths::AppDataLayout;

use super::atomic_json::write_atomic_json;

const FILE_NAME: &str = "personas.json";

fn load_personas(layout: &AppDataLayout) -> PersonaSettingsDto {
    let path = layout.config.join(FILE_NAME);
    let Ok(raw) = fs::read_to_string(&path) else {
        return PersonaSettingsDto::default();
    };
    match serde_json::from_str::<PersonaSettingsDto>(&raw) {
        Ok(settings) if settings.validate().is_ok() => settings,
        Ok(_) | Err(_) => PersonaSettingsDto::default(),
    }
}

/// Loads the persona keyed by the resolved `CharacterManifestDto.id`.
#[must_use]
pub fn load_persona(layout: &AppDataLayout, character_id: &str) -> Option<PersonaProfileDto> {
    load_personas(layout).personas.remove(character_id)
}

/// Validates every identity and atomically replaces `config/personas.json`.
///
/// # Errors
///
/// Returns a validation, serialization, or filesystem error.
pub fn save_persona_settings(
    layout: &AppDataLayout,
    settings: &PersonaSettingsDto,
) -> Result<(), String> {
    settings.validate()?;
    write_atomic_json(&layout.config, FILE_NAME, settings)
}

/// Creates a missing persona from the legacy LLM character prompt.
///
/// Existing personas win, making repeated migration calls idempotent. The
/// legacy settings are borrowed and never mutated; callers may switch their
/// read source only after this function returns `Ok`.
///
/// # Errors
///
/// Returns a validation, serialization, or filesystem error.
pub fn migrate_legacy_character_prompt(
    layout: &AppDataLayout,
    character_id: &str,
    legacy: &LlmSettingsDto,
) -> Result<PersonaProfileDto, String> {
    let mut settings = load_personas(layout);
    if let Some(existing) = settings.personas.get(character_id) {
        return Ok(existing.clone());
    }

    let mut profile = PersonaProfileDto::for_character(character_id);
    profile.free_text.clone_from(&legacy.character_prompt);
    profile.validate()?;
    settings
        .personas
        .insert(character_id.to_owned(), profile.clone());
    save_persona_settings(layout, &settings)?;
    Ok(profile)
}
