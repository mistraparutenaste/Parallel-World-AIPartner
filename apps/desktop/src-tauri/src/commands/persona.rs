//! Settings-only commands for per-character persona profiles.

use pw_contracts::PersonaProfileDto;
use pw_platform::paths::AppDataLayout;
use tauri::{AppHandle, Runtime, State};

use crate::behavior::{load_persona_checked, migrate_legacy_character_prompt, save_persona};
use crate::character::load_character_settings;
use crate::chat::load_llm_settings;

fn ensure_known_character(layout: &AppDataLayout, character_id: &str) -> Result<(), String> {
    if character_id.trim().is_empty() {
        return Err("character_id must not be empty".to_owned());
    }
    let settings = load_character_settings(layout);
    let known = [
        settings.active_character_id.as_deref(),
        settings.live2d_character_id.as_deref(),
        settings.static_image_character_id.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|known| known == character_id);
    if known {
        Ok(())
    } else {
        Err(format!("unknown character_id: {character_id}"))
    }
}

fn dark_expression_weakened(previous: &PersonaProfileDto, next: &PersonaProfileDto) -> bool {
    (previous.allow_intense_dark_expression && !next.allow_intense_dark_expression)
        || next.machiavellianism < previous.machiavellianism
        || next.narcissism < previous.narcissism
        || next.psychopathy < previous.psychopathy
        || next.sadism < previous.sadism
}

pub(crate) fn get_persona_profile_for_layout(
    layout: &AppDataLayout,
    character_id: &str,
) -> Result<PersonaProfileDto, String> {
    ensure_known_character(layout, character_id)?;
    if let Some(profile) = load_persona_checked(layout, character_id)? {
        return Ok(profile);
    }
    let legacy = load_llm_settings(layout);
    migrate_legacy_character_prompt(layout, character_id, &legacy)
}

pub(crate) fn set_persona_profile_for_layout(
    layout: &AppDataLayout,
    profile: PersonaProfileDto,
) -> Result<PersonaProfileDto, String> {
    ensure_known_character(layout, &profile.character_id)?;
    save_persona(layout, profile)
}

/// Loads or safely migrates one configured character's persona.
///
/// # Errors
///
/// Returns an identity, read, migration, or validation error.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn get_persona_profile(
    layout: State<'_, AppDataLayout>,
    character_id: String,
) -> Result<PersonaProfileDto, String> {
    get_persona_profile_for_layout(&layout, &character_id)
}

/// Validates and atomically saves one configured character's persona.
///
/// # Errors
///
/// Returns an identity, validation, existing-store, or filesystem error.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn set_persona_profile<R: Runtime>(
    app: AppHandle<R>,
    service: State<'_, crate::chat::ChatService>,
    tts: State<'_, crate::tts::TtsService>,
    layout: State<'_, AppDataLayout>,
    profile: PersonaProfileDto,
) -> Result<PersonaProfileDto, String> {
    let previous = load_persona_checked(&layout, &profile.character_id)?;
    let saved = set_persona_profile_for_layout(&layout, profile)?;
    if previous
        .as_ref()
        .is_some_and(|previous| dark_expression_weakened(previous, &saved))
    {
        service.cancel();
        tts.stop(&app);
    }
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use pw_contracts::{CharacterSettingsDto, PersonaProfileDto};
    use pw_platform::paths::AppDataLayout;

    use super::{
        dark_expression_weakened, get_persona_profile_for_layout, set_persona_profile_for_layout,
    };
    use crate::character::save_character_settings;
    use crate::chat::{default_llm_settings, save_llm_settings};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestLayout {
        layout: AppDataLayout,
    }

    impl TestLayout {
        fn new(name: &str) -> Self {
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "pw-persona-command-{name}-{}-{sequence}",
                std::process::id()
            ));
            let layout = AppDataLayout::under(root);
            layout.create_all().unwrap();
            save_character_settings(
                &layout,
                &CharacterSettingsDto {
                    active_character_id: Some("alpha".into()),
                    live2d_character_id: Some("alpha".into()),
                    static_image_character_id: Some("beta".into()),
                    ..CharacterSettingsDto::default()
                },
            )
            .unwrap();
            Self { layout }
        }
    }

    impl Drop for TestLayout {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.layout.root);
        }
    }

    #[test]
    fn get_profile_migrates_legacy_prompt_for_known_character() {
        let test = TestLayout::new("get");
        let mut llm = default_llm_settings();
        llm.character_prompt = "legacy persona".into();
        save_llm_settings(&test.layout, &llm).unwrap();

        let profile = get_persona_profile_for_layout(&test.layout, "alpha").unwrap();

        assert_eq!(profile.character_id, "alpha");
        assert_eq!(profile.free_text, "legacy persona");
        assert_eq!(profile.psychopathy, 50);
        assert!(!profile.allow_intense_dark_expression);
    }

    #[test]
    fn persona_commands_reject_unconfigured_character_id() {
        let test = TestLayout::new("unknown");
        assert!(get_persona_profile_for_layout(&test.layout, "unknown").is_err());
        assert!(
            set_persona_profile_for_layout(
                &test.layout,
                PersonaProfileDto::for_character("unknown")
            )
            .is_err()
        );
    }

    #[test]
    fn set_profile_round_trips_for_known_inactive_character() {
        let test = TestLayout::new("set");
        let mut profile = PersonaProfileDto::for_character("beta");
        profile.psychopathy = 80;

        let saved = set_persona_profile_for_layout(&test.layout, profile.clone()).unwrap();

        assert_eq!(saved, profile);
        assert_eq!(
            get_persona_profile_for_layout(&test.layout, "beta").unwrap(),
            profile
        );
    }

    #[test]
    fn lowering_any_dark_control_requires_stopping_the_old_snapshot() {
        let mut previous = PersonaProfileDto::for_character("alpha");
        previous.allow_intense_dark_expression = true;
        previous.dark_expression_acknowledgement_version =
            Some(pw_contracts::DARK_EXPRESSION_ACKNOWLEDGEMENT_VERSION);
        previous.machiavellianism = 80;
        previous.narcissism = 80;
        previous.psychopathy = 80;
        previous.sadism = 80;

        for next in [
            PersonaProfileDto {
                allow_intense_dark_expression: false,
                dark_expression_acknowledgement_version: None,
                ..previous.clone()
            },
            PersonaProfileDto {
                machiavellianism: 79,
                ..previous.clone()
            },
            PersonaProfileDto {
                narcissism: 79,
                ..previous.clone()
            },
            PersonaProfileDto {
                psychopathy: 79,
                ..previous.clone()
            },
            PersonaProfileDto {
                sadism: 79,
                ..previous.clone()
            },
        ] {
            assert!(dark_expression_weakened(&previous, &next));
        }
    }

    #[test]
    fn raising_dark_controls_does_not_interrupt_the_current_snapshot() {
        let previous = PersonaProfileDto::for_character("alpha");
        let next = PersonaProfileDto {
            machiavellianism: 51,
            narcissism: 51,
            psychopathy: 51,
            sadism: 51,
            ..previous.clone()
        };

        assert!(!dark_expression_weakened(&previous, &next));
    }
}
