//! Atomic persistence for `config/personas.json`.

use std::fs;
use std::io;

use pw_contracts::{LlmSettingsDto, PersonaProfileDto, PersonaSettingsDto};
use pw_platform::paths::AppDataLayout;

use super::atomic_json::write_atomic_json;

const FILE_NAME: &str = "personas.json";

fn read_personas(layout: &AppDataLayout) -> Result<Option<PersonaSettingsDto>, String> {
    let path = layout.config.join(FILE_NAME);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("failed to read {}: {error}", path.display())),
    };
    let settings = serde_json::from_str::<PersonaSettingsDto>(&raw)
        .map_err(|error| format!("invalid {}: {error}", path.display()))?;
    settings.validate()?;
    Ok(Some(settings))
}

fn load_personas(layout: &AppDataLayout) -> PersonaSettingsDto {
    read_personas(layout).ok().flatten().unwrap_or_default()
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
    let mut settings = read_personas(layout)?.unwrap_or_default();
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
