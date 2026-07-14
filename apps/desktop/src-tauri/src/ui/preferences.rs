use pw_contracts::{SCHEMA_VERSION, UiPreferencesDto};
use pw_platform::paths::AppDataLayout;

fn path(layout: &AppDataLayout) -> std::path::PathBuf {
    layout.config.join("ui.json")
}

#[must_use]
pub fn load_preferences(layout: &AppDataLayout) -> UiPreferencesDto {
    std::fs::read_to_string(path(layout))
        .ok()
        .and_then(|json| serde_json::from_str::<UiPreferencesDto>(&json).ok())
        .filter(|value| value.schema_version == SCHEMA_VERSION)
        .unwrap_or_default()
}

/// Persists validated UI preferences.
///
/// # Errors
///
/// Returns an error for an unsupported schema or failed serialization/write.
pub fn save_preferences(
    layout: &AppDataLayout,
    preferences: &UiPreferencesDto,
) -> Result<(), String> {
    if preferences.schema_version != SCHEMA_VERSION {
        return Err("unsupported UI preferences schema".into());
    }
    let destination = path(layout);
    let temporary = destination.with_extension("json.tmp");
    let json = serde_json::to_vec_pretty(preferences).map_err(|error| error.to_string())?;
    std::fs::write(&temporary, json).map_err(|error| error.to_string())?;
    std::fs::rename(&temporary, &destination).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{load_preferences, save_preferences};
    use pw_contracts::{ChatPlacementDto, SCHEMA_VERSION, ThemePreferenceDto, UiPreferencesDto};
    use pw_platform::paths::AppDataLayout;

    fn layout(tag: &str) -> AppDataLayout {
        let root =
            std::env::temp_dir().join(format!("pw-ui-preferences-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let layout = AppDataLayout::under(root);
        layout.create_all().unwrap();
        layout
    }

    #[test]
    fn missing_preferences_default_to_system_and_docked() {
        let layout = layout("missing");
        let value = load_preferences(&layout);
        assert_eq!(value.schema_version, SCHEMA_VERSION);
        assert_eq!(value.theme, ThemePreferenceDto::System);
        assert_eq!(value.chat_placement, ChatPlacementDto::Docked);
        let _ = std::fs::remove_dir_all(layout.root);
    }

    #[test]
    fn saved_preferences_round_trip() {
        let layout = layout("round-trip");
        let value = UiPreferencesDto {
            schema_version: SCHEMA_VERSION,
            theme: ThemePreferenceDto::Dark,
            chat_placement: ChatPlacementDto::Popped,
        };
        save_preferences(&layout, &value).unwrap();
        assert_eq!(load_preferences(&layout), value);

        let updated = UiPreferencesDto {
            schema_version: SCHEMA_VERSION,
            theme: ThemePreferenceDto::Light,
            chat_placement: ChatPlacementDto::Docked,
        };
        save_preferences(&layout, &updated).unwrap();
        assert_eq!(load_preferences(&layout), updated);
        let _ = std::fs::remove_dir_all(layout.root);
    }

    #[test]
    fn corrupt_or_unknown_preferences_fall_back_to_defaults() {
        let layout = layout("invalid");
        let path = layout.config.join("ui.json");
        std::fs::write(&path, "{not-json").unwrap();
        assert_eq!(load_preferences(&layout), UiPreferencesDto::default());
        std::fs::write(
            &path,
            r#"{"schema_version":999,"theme":"dark","chat_placement":"popped"}"#,
        )
        .unwrap();
        assert_eq!(load_preferences(&layout), UiPreferencesDto::default());
        let _ = std::fs::remove_dir_all(layout.root);
    }
}
