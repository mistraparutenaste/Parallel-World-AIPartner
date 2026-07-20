//! Global character selection and behavior settings persistence.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use pw_contracts::{CHARACTER_SETTINGS_SCHEMA_VERSION, CharacterSettingsDto};
use pw_platform::{diagnostics::atomic_replace, paths::AppDataLayout};

const SETTINGS_FILE_NAME: &str = "character-settings.json";
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

/// Validates the optional expression idle timeout.
///
/// `None` disables the reset. Enabled values must be from 10 through
/// 600 seconds, inclusive.
///
/// # Errors
///
/// Returns an error when the enabled value is outside that range.
pub fn validate_idle_timeout(timeout_seconds: Option<u32>) -> Result<(), String> {
    match timeout_seconds {
        None | Some(10..=600) => Ok(()),
        Some(value) => Err(format!(
            "expression idle timeout must be null or between 10 and 600 seconds: {value}"
        )),
    }
}

/// Validates the character window scale percentage.
pub fn validate_character_size(size_percent: u16) -> Result<(), String> {
    if (50..=200).contains(&size_percent) {
        Ok(())
    } else {
        Err(format!(
            "character size must be between 50 and 200 percent: {size_percent}"
        ))
    }
}

fn validate_settings(settings: &CharacterSettingsDto) -> Result<(), String> {
    if settings.schema_version != CHARACTER_SETTINGS_SCHEMA_VERSION {
        return Err(format!(
            "unsupported character settings schema version: {}",
            settings.schema_version
        ));
    }
    validate_idle_timeout(settings.expression_idle_timeout_seconds)
        .and_then(|()| validate_character_size(settings.character_size_percent))
}

fn migrate_settings(mut settings: CharacterSettingsDto) -> Result<CharacterSettingsDto, String> {
    match settings.schema_version {
        1 => {
            settings.schema_version = CHARACTER_SETTINGS_SCHEMA_VERSION;
            settings.live2d_character_id = None;
            settings.static_image_character_id = None;
        }
        2 => {
            settings.schema_version = CHARACTER_SETTINGS_SCHEMA_VERSION;
        }
        CHARACTER_SETTINGS_SCHEMA_VERSION => {}
        version => {
            return Err(format!(
                "unsupported character settings schema version: {version}"
            ));
        }
    }
    validate_idle_timeout(settings.expression_idle_timeout_seconds)?;
    validate_character_size(settings.character_size_percent)?;
    Ok(settings)
}

/// Loads persisted character settings, falling back to defaults when
/// the file is absent, corrupt, or invalid.
#[must_use]
pub fn load_character_settings(layout: &AppDataLayout) -> CharacterSettingsDto {
    let path = layout.config.join(SETTINGS_FILE_NAME);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return CharacterSettingsDto::default();
        }
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "failed to read character settings; using defaults");
            return CharacterSettingsDto::default();
        }
    };

    match serde_json::from_str::<CharacterSettingsDto>(&raw) {
        Ok(settings) => match migrate_settings(settings) {
            Ok(settings) => settings,
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "invalid character settings; using defaults");
                CharacterSettingsDto::default()
            }
        },
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "failed to parse character settings; using defaults");
            CharacterSettingsDto::default()
        }
    }
}

/// Persists validated settings through a flushed sibling temporary
/// file followed by an atomic rename.
///
/// # Errors
///
/// Returns a validation, serialization, or filesystem error.
pub fn save_character_settings(
    layout: &AppDataLayout,
    settings: &CharacterSettingsDto,
) -> Result<(), String> {
    validate_settings(settings)?;
    fs::create_dir_all(&layout.config).map_err(|error| error.to_string())?;

    let destination = layout.config.join(SETTINGS_FILE_NAME);
    let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
    let temporary = layout.config.join(format!(
        ".{SETTINGS_FILE_NAME}.{}.{sequence}.tmp",
        std::process::id()
    ));
    let serialized = serde_json::to_vec_pretty(settings).map_err(|error| error.to_string())?;

    let write_result = (|| -> Result<(), String> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        file.write_all(&serialized)
            .map_err(|error| error.to_string())?;
        file.write_all(b"\n").map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        drop(file);
        atomic_replace(&temporary, &destination).map_err(|error| error.to_string())?;
        Ok(())
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
}

/// Replaces only the idle timeout while preserving character
/// selection.
///
/// # Errors
///
/// Returns an error when the timeout is outside the supported range.
pub fn with_expression_idle_timeout(
    mut settings: CharacterSettingsDto,
    timeout_seconds: Option<u32>,
) -> Result<CharacterSettingsDto, String> {
    validate_idle_timeout(timeout_seconds)?;
    settings.schema_version = CHARACTER_SETTINGS_SCHEMA_VERSION;
    settings.expression_idle_timeout_seconds = timeout_seconds;
    Ok(settings)
}

/// Replaces only the character window scale while preserving all other settings.
pub fn with_character_size(
    mut settings: CharacterSettingsDto,
    size_percent: u16,
) -> Result<CharacterSettingsDto, String> {
    validate_character_size(size_percent)?;
    settings.schema_version = CHARACTER_SETTINGS_SCHEMA_VERSION;
    settings.character_size_percent = size_percent;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use pw_contracts::{CHARACTER_SETTINGS_SCHEMA_VERSION, CharacterSettingsDto};
    use pw_platform::paths::AppDataLayout;

    use super::{
        load_character_settings, save_character_settings, validate_character_size,
        validate_idle_timeout, with_character_size, with_expression_idle_timeout,
    };

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestLayout {
        layout: AppDataLayout,
        root: PathBuf,
    }

    impl TestLayout {
        fn new(name: &str) -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "pw-character-settings-{name}-{}-{sequence}",
                std::process::id()
            ));
            let layout = AppDataLayout::under(root.clone());
            layout.create_all().expect("create test app-data layout");
            Self { layout, root }
        }

        fn settings_path(&self) -> PathBuf {
            self.layout.config.join("character-settings.json")
        }
    }

    impl Drop for TestLayout {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn settings(active_character_id: Option<&str>, timeout: Option<u32>) -> CharacterSettingsDto {
        CharacterSettingsDto {
            schema_version: CHARACTER_SETTINGS_SCHEMA_VERSION,
            active_character_id: active_character_id.map(str::to_owned),
            expression_idle_timeout_seconds: timeout,
            ..CharacterSettingsDto::default()
        }
    }

    #[test]
    fn missing_file_loads_default_settings() {
        let test = TestLayout::new("missing");

        assert_eq!(
            load_character_settings(&test.layout),
            CharacterSettingsDto::default()
        );
    }

    #[test]
    fn null_timeout_round_trips() {
        let test = TestLayout::new("null-round-trip");
        let expected = settings(Some("epsilon"), None);

        save_character_settings(&test.layout, &expected).expect("save null timeout");

        assert_eq!(load_character_settings(&test.layout), expected);
    }

    #[test]
    fn timeout_bounds_round_trip() {
        for timeout in [10, 600] {
            let test = TestLayout::new(&format!("bounded-{timeout}"));
            let expected = settings(Some("epsilon"), Some(timeout));

            save_character_settings(&test.layout, &expected).expect("save bounded timeout");

            assert_eq!(load_character_settings(&test.layout), expected);
        }
    }

    #[test]
    fn all_character_selections_round_trip() {
        let test = TestLayout::new("all-selections-round-trip");
        let expected = serde_json::json!({
            "schema_version": 3,
            "active_character_id": "active-static",
            "live2d_character_id": "remembered-live2d",
            "static_image_character_id": "remembered-static",
            "expression_idle_timeout_seconds": 30,
            "character_size_percent": 100,
        });
        let settings = serde_json::from_value(expected.clone()).expect("deserialize v3 settings");

        save_character_settings(&test.layout, &settings).expect("save v3 settings");

        assert_eq!(
            serde_json::to_value(load_character_settings(&test.layout)).unwrap(),
            expected
        );
    }

    #[test]
    fn schema_v1_migrates_to_v3_without_remembered_sources() {
        let test = TestLayout::new("schema-v1-migration");
        std::fs::write(
            test.settings_path(),
            r#"{"schema_version":1,"active_character_id":"epsilon","expression_idle_timeout_seconds":30}"#,
        )
        .expect("write v1 settings");

        assert_eq!(
            serde_json::to_value(load_character_settings(&test.layout)).unwrap(),
            serde_json::json!({
                "schema_version": 3,
                "active_character_id": "epsilon",
                "live2d_character_id": null,
                "static_image_character_id": null,
                "expression_idle_timeout_seconds": 30,
                "character_size_percent": 100,
            })
        );
    }

    #[test]
    fn timeout_outside_bounds_is_rejected() {
        for timeout in [9, 601] {
            assert!(validate_idle_timeout(Some(timeout)).is_err(), "{timeout}");
        }
        assert!(validate_idle_timeout(None).is_ok());
        assert!(validate_idle_timeout(Some(10)).is_ok());
        assert!(validate_idle_timeout(Some(600)).is_ok());
    }

    #[test]
    fn character_size_bounds_are_validated() {
        for size in [49, 201] {
            assert!(validate_character_size(size).is_err(), "{size}");
        }
        for size in [50, 100, 200] {
            assert!(validate_character_size(size).is_ok(), "{size}");
        }
    }

    #[test]
    fn changing_character_size_preserves_character_selection_and_timeout() {
        let original = settings(Some("epsilon"), Some(30));
        let updated = with_character_size(original.clone(), 150).expect("set character size");

        assert_eq!(updated.active_character_id, original.active_character_id);
        assert_eq!(
            updated.expression_idle_timeout_seconds,
            original.expression_idle_timeout_seconds
        );
        assert_eq!(updated.character_size_percent, 150);
        assert_eq!(updated.schema_version, CHARACTER_SETTINGS_SCHEMA_VERSION);
    }

    #[test]
    fn corrupt_json_loads_default_settings() {
        let test = TestLayout::new("corrupt");
        std::fs::write(test.settings_path(), b"{not-json").expect("write corrupt settings");

        assert_eq!(
            load_character_settings(&test.layout),
            CharacterSettingsDto::default()
        );
    }

    #[test]
    fn invalid_schema_or_timeout_loads_default_settings() {
        for (name, raw) in [
            (
                "schema",
                r#"{"schema_version":4,"active_character_id":"epsilon","expression_idle_timeout_seconds":20}"#,
            ),
            (
                "timeout",
                r#"{"schema_version":2,"active_character_id":"epsilon","expression_idle_timeout_seconds":9}"#,
            ),
        ] {
            let test = TestLayout::new(name);
            std::fs::write(test.settings_path(), raw).expect("write invalid settings");

            assert_eq!(
                load_character_settings(&test.layout),
                CharacterSettingsDto::default(),
                "{name}"
            );
        }
    }

    #[test]
    fn changing_timeout_preserves_active_character_id() {
        let original = serde_json::from_value(serde_json::json!({
            "schema_version": 2,
            "active_character_id": "epsilon-static",
            "live2d_character_id": "epsilon-live2d",
            "static_image_character_id": "epsilon-static",
            "expression_idle_timeout_seconds": 20,
        }))
        .expect("deserialize settings with remembered sources");

        let updated = with_expression_idle_timeout(original, None).expect("disable idle timeout");

        assert_eq!(
            serde_json::to_value(updated).unwrap(),
            serde_json::json!({
                "schema_version": 3,
                "active_character_id": "epsilon-static",
                "live2d_character_id": "epsilon-live2d",
                "static_image_character_id": "epsilon-static",
                "expression_idle_timeout_seconds": null,
                "character_size_percent": 100,
            })
        );
    }

    #[test]
    fn save_atomically_replaces_existing_settings_without_temp_artifacts() {
        let test = TestLayout::new("atomic-replace");
        save_character_settings(&test.layout, &settings(Some("old"), Some(10)))
            .expect("save initial settings");

        let replacement = settings(Some("new"), Some(600));
        save_character_settings(&test.layout, &replacement).expect("replace settings");

        assert_eq!(load_character_settings(&test.layout), replacement);
        let entries = std::fs::read_dir(&test.layout.config)
            .expect("read config directory")
            .map(|entry| entry.expect("read config entry").file_name())
            .collect::<Vec<_>>();
        assert_eq!(entries, ["character-settings.json"]);
    }
}
