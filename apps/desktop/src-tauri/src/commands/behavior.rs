//! User-wide context-aware companion settings commands.

use pw_contracts::{
    BEHAVIOR_SETTINGS_CHANGED_EVENT, BEHAVIOR_SETTINGS_SCHEMA_VERSION,
    BehaviorSettingsChangedEventDto, BehaviorSettingsDto,
};
use pw_platform::paths::AppDataLayout;
use tauri::{AppHandle, Emitter, Runtime, State};

use crate::behavior::{load_behavior_settings_checked, save_behavior_settings};

pub(crate) fn get_behavior_settings_for_layout(
    layout: &AppDataLayout,
) -> Result<BehaviorSettingsDto, String> {
    load_behavior_settings_checked(layout).map_err(|error| error.to_string())
}

pub(crate) fn set_behavior_settings_for_layout(
    layout: &AppDataLayout,
    settings: BehaviorSettingsDto,
) -> Result<BehaviorSettingsDto, String> {
    save_behavior_settings(layout, &settings)?;
    Ok(settings)
}

/// Loads the validated user-wide companion settings.
///
/// # Errors
///
/// Returns a stable read or validation error without exposing file contents.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn get_behavior_settings(
    layout: State<'_, AppDataLayout>,
) -> Result<BehaviorSettingsDto, String> {
    get_behavior_settings_for_layout(&layout)
}

/// Atomically saves the user-wide companion settings and broadcasts the snapshot.
///
/// # Errors
///
/// Returns a validation or persistence error.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn set_behavior_settings<R: Runtime>(
    app: AppHandle<R>,
    layout: State<'_, AppDataLayout>,
    settings: BehaviorSettingsDto,
) -> Result<BehaviorSettingsDto, String> {
    let settings = set_behavior_settings_for_layout(&layout, settings)?;
    if let Err(error) = app.emit(
        BEHAVIOR_SETTINGS_CHANGED_EVENT,
        BehaviorSettingsChangedEventDto {
            schema_version: BEHAVIOR_SETTINGS_SCHEMA_VERSION,
            settings: settings.clone(),
        },
    ) {
        tracing::warn!(%error, "failed to emit behavior settings");
    }
    Ok(settings)
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
/// Returns the latest mode resolved by the behavior worker.
///
/// # Errors
///
/// Returns an error while the background worker is still resolving its first
/// mode snapshot.
pub fn get_active_mode(
    runtime: State<'_, crate::behavior::BehaviorRuntimeService>,
) -> Result<pw_contracts::ActiveModeDto, String> {
    runtime
        .active_mode()
        .ok_or_else(|| "behavior runtime is starting".to_owned())
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
#[must_use]
pub fn get_activity_collection_health(
    runtime: State<'_, crate::behavior::BehaviorRuntimeService>,
) -> pw_contracts::ActivityCollectionHealthEventDto {
    runtime.collection_health()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use pw_contracts::BehaviorSettingsDto;
    use pw_platform::paths::AppDataLayout;

    use super::{get_behavior_settings_for_layout, set_behavior_settings_for_layout};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestLayout {
        layout: AppDataLayout,
    }

    impl TestLayout {
        fn new(name: &str) -> Self {
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "pw-behavior-command-{name}-{}-{sequence}",
                std::process::id()
            ));
            let layout = AppDataLayout::under(root);
            layout.create_all().unwrap();
            Self { layout }
        }
    }

    impl Drop for TestLayout {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.layout.root);
        }
    }

    #[test]
    fn missing_settings_return_fail_closed_defaults() {
        let test = TestLayout::new("missing");

        let settings = get_behavior_settings_for_layout(&test.layout).unwrap();

        assert_eq!(settings, BehaviorSettingsDto::default());
        assert!(!settings.proactive_master_enabled);
        assert!(!settings.collection_enabled);
    }

    #[test]
    fn set_settings_round_trips_the_validated_snapshot() {
        let test = TestLayout::new("set");
        let settings = BehaviorSettingsDto {
            proactive_master_enabled: true,
            proactive_snoozed_until: Some(1_800_000_000),
            ..BehaviorSettingsDto::default()
        };

        let saved = set_behavior_settings_for_layout(&test.layout, settings.clone()).unwrap();

        assert_eq!(saved, settings);
        assert_eq!(
            get_behavior_settings_for_layout(&test.layout).unwrap(),
            settings
        );
    }

    #[test]
    fn invalid_existing_settings_are_not_overwritten_by_a_read() {
        let test = TestLayout::new("invalid");
        let path = test.layout.config.join("behavior.json");
        fs::write(&path, b"{private-invalid").unwrap();
        let before = fs::read(&path).unwrap();

        let error = get_behavior_settings_for_layout(&test.layout).unwrap_err();

        assert_eq!(error, "behavior settings are invalid");
        assert_eq!(fs::read(path).unwrap(), before);
    }
}
