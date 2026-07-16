use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use parallel_world_desktop::behavior::{
    load_behavior_settings, load_behavior_settings_checked, load_persona, load_persona_checked,
    migrate_legacy_character_prompt, save_behavior_settings, save_persona, save_persona_settings,
};
use parallel_world_desktop::chat::default_llm_settings;
use pw_contracts::{BehaviorSettingsDto, PersonaProfileDto, PersonaSettingsDto};
use pw_platform::paths::AppDataLayout;

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TestLayout {
    layout: AppDataLayout,
    root: PathBuf,
}

#[test]
fn behavior_legacy_prompt_migration_is_idempotent_and_preserves_legacy_settings() {
    let test = TestLayout::new("legacy-migration");
    let mut llm = default_llm_settings();
    llm.character_prompt = "legacy persona prompt".to_owned();

    let migrated = migrate_legacy_character_prompt(&test.layout, "epsilon", &llm)
        .expect("migrate legacy prompt");
    assert_eq!(migrated.free_text, "legacy persona prompt");
    assert_eq!(llm.character_prompt, "legacy persona prompt");

    let mut changed_legacy = llm.clone();
    changed_legacy.character_prompt = "must not overwrite persona".to_owned();
    let second = migrate_legacy_character_prompt(&test.layout, "epsilon", &changed_legacy)
        .expect("repeat migration");
    assert_eq!(second, migrated);
    assert_eq!(
        load_persona(&test.layout, "epsilon")
            .expect("stored persona")
            .free_text,
        "legacy persona prompt"
    );
}

#[test]
fn behavior_legacy_migration_rejects_invalid_existing_store_without_overwrite() {
    let valid_profile = PersonaProfileDto::for_character("epsilon");
    let mut invalid_slider = valid_profile.clone();
    invalid_slider.initiative = 101;

    for (name, raw) in [
        ("corrupt", "{not-json".to_owned()),
        (
            "wrong-schema",
            r#"{"schema_version":99,"personas":{}}"#.to_owned(),
        ),
        (
            "key-mismatch",
            serde_json::json!({
                "schema_version": 1,
                "personas": { "wrong": valid_profile }
            })
            .to_string(),
        ),
        (
            "invalid-slider",
            serde_json::json!({
                "schema_version": 1,
                "personas": { "epsilon": invalid_slider }
            })
            .to_string(),
        ),
    ] {
        let test = TestLayout::new(name);
        let path = test.layout.config.join("personas.json");
        std::fs::write(&path, raw.as_bytes()).expect("write invalid personas");
        let before = std::fs::read(&path).expect("read original personas");
        let legacy = default_llm_settings();

        assert!(
            migrate_legacy_character_prompt(&test.layout, "epsilon", &legacy).is_err(),
            "{name}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), before, "{name}");
    }
}

#[test]
fn behavior_persona_store_rejects_duplicate_character_identities() {
    let test = TestLayout::new("persona-duplicate");
    let profile = serde_json::to_string(&PersonaProfileDto::for_character("epsilon"))
        .expect("serialize profile");
    let raw = [
        r#"{"schema_version":1,"personas":{"epsilon":"#,
        &profile,
        r#","epsilon":"#,
        &profile,
        "}}",
    ]
    .concat();
    std::fs::write(test.layout.config.join("personas.json"), raw).expect("write duplicate file");

    assert_eq!(load_persona(&test.layout, "epsilon"), None);
}

#[test]
fn behavior_persona_store_rejects_key_mismatch_and_round_trips_atomically() {
    let test = TestLayout::new("persona-atomic");
    let mut file = PersonaSettingsDto::default();
    file.personas.insert(
        "wrong-key".to_owned(),
        PersonaProfileDto::for_character("epsilon"),
    );
    assert!(save_persona_settings(&test.layout, &file).is_err());

    file.personas.clear();
    let mut profile = PersonaProfileDto::for_character("epsilon");
    profile.name = "Epsilon".to_owned();
    file.personas.insert("epsilon".to_owned(), profile.clone());
    save_persona_settings(&test.layout, &file).expect("save personas");

    assert_eq!(load_persona(&test.layout, "epsilon"), Some(profile));
    let entries = std::fs::read_dir(&test.layout.config)
        .expect("read config")
        .map(|entry| entry.expect("read entry").file_name())
        .collect::<Vec<_>>();
    assert_eq!(entries, ["personas.json"]);
}

#[test]
fn behavior_checked_persona_load_migrates_v1_in_memory_without_writing() {
    let test = TestLayout::new("persona-v1-checked");
    let path = test.layout.config.join("personas.json");
    let mut profile = serde_json::to_value(PersonaProfileDto::for_character("epsilon")).unwrap();
    let object = profile.as_object_mut().unwrap();
    for field in [
        "machiavellianism",
        "narcissism",
        "psychopathy",
        "allow_intense_dark_expression",
        "dark_expression_acknowledgement_version",
    ] {
        object.remove(field);
    }
    let raw = serde_json::json!({
        "schema_version": 1,
        "personas": { "epsilon": profile }
    })
    .to_string();
    std::fs::write(&path, raw.as_bytes()).unwrap();
    let before = std::fs::read(&path).unwrap();

    let loaded = load_persona_checked(&test.layout, "epsilon")
        .expect("v1 is readable")
        .expect("profile exists");

    assert_eq!(loaded.machiavellianism, 50);
    assert!(!loaded.allow_intense_dark_expression);
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn behavior_checked_persona_load_reports_invalid_without_overwrite() {
    let test = TestLayout::new("persona-invalid-checked");
    let path = test.layout.config.join("personas.json");
    std::fs::write(&path, b"{private-invalid").unwrap();
    let before = std::fs::read(&path).unwrap();

    assert!(load_persona_checked(&test.layout, "epsilon").is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn behavior_single_persona_save_preserves_other_characters() {
    let test = TestLayout::new("persona-single-save");
    let mut settings = PersonaSettingsDto::default();
    settings.personas.insert(
        "alpha".to_owned(),
        PersonaProfileDto::for_character("alpha"),
    );
    settings
        .personas
        .insert("beta".to_owned(), PersonaProfileDto::for_character("beta"));
    save_persona_settings(&test.layout, &settings).unwrap();
    let mut alpha = settings.personas["alpha"].clone();
    alpha.psychopathy = 80;

    let saved = save_persona(&test.layout, alpha.clone()).expect("save one profile");

    assert_eq!(saved, alpha);
    assert_eq!(load_persona(&test.layout, "alpha"), Some(alpha));
    assert_eq!(
        load_persona(&test.layout, "beta"),
        Some(settings.personas["beta"].clone())
    );
}

#[test]
fn behavior_settings_round_trip_atomically_without_temp_artifacts() {
    let test = TestLayout::new("behavior-atomic");
    let expected = BehaviorSettingsDto {
        retention_days: 45,
        ..BehaviorSettingsDto::default()
    };

    save_behavior_settings(&test.layout, &expected).expect("save behavior settings");
    assert_eq!(load_behavior_settings(&test.layout), expected);

    let entries = std::fs::read_dir(&test.layout.config)
        .expect("read config")
        .map(|entry| entry.expect("read entry").file_name())
        .collect::<Vec<_>>();
    assert_eq!(entries, ["behavior.json"]);
}

impl TestLayout {
    fn new(name: &str) -> Self {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "pw-behavior-store-{name}-{}-{sequence}",
            std::process::id()
        ));
        let layout = AppDataLayout::under(root.clone());
        layout.create_all().expect("create test layout");
        Self { layout, root }
    }
}

impl Drop for TestLayout {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn behavior_missing_files_return_privacy_safe_defaults() {
    let test = TestLayout::new("missing");

    let behavior = load_behavior_settings(&test.layout);
    assert_eq!(behavior, BehaviorSettingsDto::default());
    assert!(!behavior.collection_enabled);
    assert_eq!(load_persona(&test.layout, "epsilon"), None);
}

#[test]
fn corrupt_or_wrong_schema_behavior_files_return_collection_off_defaults() {
    for (name, raw) in [
        ("corrupt", "{not-json".to_owned()),
        ("schema", {
            let mut value = serde_json::to_value(BehaviorSettingsDto::default()).unwrap();
            value["schema_version"] = serde_json::json!(99);
            value.to_string()
        }),
    ] {
        let test = TestLayout::new(name);
        std::fs::write(test.layout.config.join("behavior.json"), raw).expect("write behavior");
        let loaded = load_behavior_settings(&test.layout);
        assert_eq!(loaded, BehaviorSettingsDto::default(), "{name}");
        assert!(!loaded.collection_enabled);
    }
}

#[test]
fn behavior_checked_loader_distinguishes_missing_from_invalid_without_leaking_content() {
    const SECRET: &str = "private-invalid-settings-sentinel";
    let missing = TestLayout::new("checked-missing");
    assert_eq!(
        load_behavior_settings_checked(&missing.layout).expect("missing means safe defaults"),
        BehaviorSettingsDto::default()
    );

    let corrupt = TestLayout::new("checked-corrupt");
    std::fs::write(
        corrupt.layout.config.join("behavior.json"),
        format!("{{not-json-{SECRET}"),
    )
    .unwrap();
    let error = load_behavior_settings_checked(&corrupt.layout).expect_err("invalid is explicit");
    assert!(!error.to_string().contains(SECRET));
}

#[test]
fn behavior_saves_reject_wrong_schema_and_invalid_ranges() {
    let test = TestLayout::new("invalid-save");
    let wrong_schema = BehaviorSettingsDto {
        schema_version: 99,
        ..BehaviorSettingsDto::default()
    };
    assert!(save_behavior_settings(&test.layout, &wrong_schema).is_err());
    let invalid_retention = BehaviorSettingsDto {
        retention_days: 0,
        ..BehaviorSettingsDto::default()
    };
    assert!(save_behavior_settings(&test.layout, &invalid_retention).is_err());

    let mut persona = PersonaProfileDto::for_character("epsilon");
    persona.initiative = 101;
    let mut personas = PersonaSettingsDto::default();
    personas.personas.insert("epsilon".to_owned(), persona);
    assert!(save_persona_settings(&test.layout, &personas).is_err());
}

#[test]
fn behavior_corrupt_wrong_schema_or_mismatched_persona_files_return_none() {
    for (name, raw) in [
        ("corrupt-persona", "{not-json".to_owned()),
        (
            "persona-schema",
            r#"{"schema_version":99,"personas":{}}"#.to_owned(),
        ),
        ("persona-mismatch", {
            let profile =
                serde_json::to_string(&PersonaProfileDto::for_character("epsilon")).unwrap();
            [
                r#"{"schema_version":1,"personas":{"wrong":"#,
                &profile,
                "}}",
            ]
            .concat()
        }),
    ] {
        let test = TestLayout::new(name);
        std::fs::write(test.layout.config.join("personas.json"), raw).expect("write personas");
        assert_eq!(load_persona(&test.layout, "epsilon"), None, "{name}");
    }
}
