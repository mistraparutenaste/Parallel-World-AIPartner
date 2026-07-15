use pw_contracts::{
    ACTIVITY_SESSION_SCHEMA_VERSION, ActiveModeChangedEventDto, ActiveModeDto, ActiveModeSourceDto,
    ActivitySessionDto, ActivitySessionPageDto, BEHAVIOR_SETTINGS_SCHEMA_VERSION,
    BehaviorSettingsDto, CompanionModeDto, ConsentStateDto, ExclusionRuleDto, PersonaProfileDto,
};
use ts_rs::{Config, TS};

#[test]
fn behavior_settings_defaults_are_private_and_rate_limited() {
    let settings = BehaviorSettingsDto::default();

    assert_eq!(settings.schema_version, BEHAVIOR_SETTINGS_SCHEMA_VERSION);
    assert_eq!(settings.consent, ConsentStateDto::Pending);
    assert!(!settings.collection_enabled);
    assert_eq!(settings.retention_days, 30);
    assert_eq!(settings.frequency.minimum_interval_minutes, 15);
    assert_eq!(settings.frequency.max_per_hour, 3);
    assert_eq!(settings.frequency.max_per_day, 16);
    assert_eq!(settings.triggers.return_after_minutes, 10);
    assert_eq!(settings.triggers.long_session_minutes, 60);
    assert_eq!(settings.triggers.category_change_minutes, 10);
    assert_eq!(settings.manual_mode_override, None);
    assert_eq!(settings.activation.fullscreen.mode, CompanionModeDto::Focus);
    assert_eq!(settings.shortcuts.push_to_talk, "Ctrl+Alt+Space");
    assert_eq!(settings.shortcuts.toggle_mute, "Ctrl+Alt+M");
    assert_eq!(settings.shortcuts.open_control_center, "Ctrl+Alt+P");
    assert_eq!(settings.shortcuts.toggle_character, "Ctrl+Alt+C");
    assert_eq!(settings.shortcuts.cycle_mode, "Ctrl+Alt+F");
    assert!(settings.profiles.normal.proactive_enabled);
    assert!(settings.profiles.normal.tts_enabled);
    assert!(settings.profiles.normal.character_enabled);
    assert!(!settings.profiles.normal.notifications_enabled);
    assert_eq!(settings.profiles.normal.volume, 1.0);
    for profile in [&settings.profiles.focus, &settings.profiles.night] {
        assert!(!profile.proactive_enabled);
        assert!(!profile.tts_enabled);
        assert!(!profile.character_enabled);
        assert!(!profile.notifications_enabled);
        assert_eq!(profile.volume, 0.0);
    }
    assert_eq!(
        serde_json::to_value(settings.consent).unwrap(),
        serde_json::json!("pending")
    );
}

#[test]
fn active_mode_event_is_versioned_and_uses_snake_case_enums() {
    let payload = ActiveModeChangedEventDto {
        schema_version: BEHAVIOR_SETTINGS_SCHEMA_VERSION,
        active_mode: ActiveModeDto {
            schema_version: BEHAVIOR_SETTINGS_SCHEMA_VERSION,
            mode: CompanionModeDto::Night,
            source: ActiveModeSourceDto::Fullscreen,
            manual_override: None,
        },
    };

    assert_eq!(
        serde_json::to_value(payload).unwrap(),
        serde_json::json!({
            "schema_version": 1,
            "active_mode": {
                "schema_version": 1,
                "mode": "night",
                "source": "fullscreen",
                "manual_override": null,
            }
        })
    );
}

#[test]
fn activity_contract_exposes_all_numeric_fields_as_typescript_numbers() {
    let config = Config::default();
    let declaration = ActivitySessionDto::decl(&config);
    for field in ["id", "started_at", "ended_at", "duration_seconds"] {
        assert!(
            declaration.contains(&format!("{field}: number"))
                || declaration.contains(&format!("{field}: number | null")),
            "missing numeric annotation for {field}: {declaration}"
        );
    }

    let page = ActivitySessionPageDto {
        schema_version: ACTIVITY_SESSION_SCHEMA_VERSION,
        sessions: Vec::new(),
        next_before_id: Some(42),
    };
    assert_eq!(serde_json::to_value(page).unwrap()["next_before_id"], 42);
    assert!(ActivitySessionPageDto::decl(&config).contains("next_before_id: number | null"));
}

#[test]
fn persona_sliders_are_bounded_and_serialize_with_snake_case_names() {
    let profile = PersonaProfileDto::for_character("epsilon");
    let json = serde_json::to_value(&profile).expect("serialize persona");
    assert_eq!(json["character_id"], "epsilon");
    assert!(json.get("first_person_pronoun").is_some());
    assert!(profile.validate().is_ok());

    for slider in [
        "initiative",
        "closeness",
        "humor",
        "response_length",
        "emotional_expression",
        "reaction_interval",
    ] {
        let mut invalid = profile.clone();
        match slider {
            "initiative" => invalid.initiative = 101,
            "closeness" => invalid.closeness = 101,
            "humor" => invalid.humor = 101,
            "response_length" => invalid.response_length = 101,
            "emotional_expression" => invalid.emotional_expression = 101,
            "reaction_interval" => invalid.reaction_interval = 101,
            _ => unreachable!(),
        }
        assert!(invalid.validate().is_err(), "{slider}");
    }
}

#[test]
fn behavior_settings_reject_invalid_transport_values() {
    let mut settings = BehaviorSettingsDto::default();
    settings.retention_days = 0;
    assert!(settings.validate().is_err());

    let mut settings = BehaviorSettingsDto::default();
    settings.frequency.minimum_interval_minutes = 0;
    assert!(settings.validate().is_err());

    let mut settings = BehaviorSettingsDto::default();
    settings.frequency.max_per_hour = 0;
    assert!(settings.validate().is_err());

    let mut settings = BehaviorSettingsDto::default();
    settings.frequency.max_per_day = 0;
    assert!(settings.validate().is_err());

    for invalid_volume in [-0.1, 1.1, f32::NAN] {
        let mut settings = BehaviorSettingsDto::default();
        settings.profiles.normal.volume = invalid_volume;
        assert!(settings.validate().is_err(), "{invalid_volume}");
    }
}

#[test]
fn behavior_activity_exclusions_are_bounded_and_nonempty() {
    for exclusion in [
        ExclusionRuleDto {
            app_id: None,
            title_pattern: None,
        },
        ExclusionRuleDto {
            app_id: Some(String::new()),
            title_pattern: None,
        },
        ExclusionRuleDto {
            app_id: None,
            title_pattern: Some(String::new()),
        },
        ExclusionRuleDto {
            app_id: Some("a".repeat(261)),
            title_pattern: None,
        },
        ExclusionRuleDto {
            app_id: None,
            title_pattern: Some("t".repeat(129)),
        },
    ] {
        let mut settings = BehaviorSettingsDto::default();
        settings.exclusions.push(exclusion);
        assert!(settings.validate().is_err());
    }

    let mut settings = BehaviorSettingsDto::default();
    settings.exclusions.push(ExclusionRuleDto {
        app_id: Some("Code.exe".to_owned()),
        title_pattern: Some("private project".to_owned()),
    });
    assert!(settings.validate().is_ok());
}
