#![allow(clippy::field_reassign_with_default, clippy::float_cmp)]

use pw_contracts::{
    ACTIVITY_SESSION_SCHEMA_VERSION, ActiveModeChangedEventDto, ActiveModeDto, ActiveModeSourceDto,
    ActivitySessionDto, ActivitySessionPageDto, AppActivationRuleDto,
    BEHAVIOR_SETTINGS_SCHEMA_VERSION, BehaviorSettingsDto, CompanionModeDto, ConsentStateDto,
    DARK_EXPRESSION_ACKNOWLEDGEMENT_VERSION, ExclusionRuleDto, PERSONA_SETTINGS_SCHEMA_VERSION,
    PersonaProfileDto, PersonaSettingsDto, ScheduleActivationRuleDto,
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
fn persona_v2_defaults_dark_traits_and_intense_expression_safely() {
    let profile = PersonaProfileDto::for_character("epsilon");

    assert_eq!(PERSONA_SETTINGS_SCHEMA_VERSION, 2);
    assert_eq!(profile.machiavellianism, 50);
    assert_eq!(profile.narcissism, 50);
    assert_eq!(profile.psychopathy, 50);
    assert!(!profile.allow_intense_dark_expression);
    assert_eq!(profile.dark_expression_acknowledgement_version, None);
}

#[test]
fn persona_rejects_out_of_range_dark_traits() {
    for trait_name in ["machiavellianism", "narcissism", "psychopathy"] {
        let mut profile = PersonaProfileDto::for_character("epsilon");
        match trait_name {
            "machiavellianism" => profile.machiavellianism = 101,
            "narcissism" => profile.narcissism = 101,
            "psychopathy" => profile.psychopathy = 101,
            _ => unreachable!(),
        }
        assert!(profile.validate().is_err(), "{trait_name}");
    }
}

#[test]
fn intense_dark_expression_requires_current_acknowledgement() {
    let mut profile = PersonaProfileDto::for_character("epsilon");
    profile.allow_intense_dark_expression = true;
    assert!(profile.validate().is_err());

    profile.dark_expression_acknowledgement_version = Some(DARK_EXPRESSION_ACKNOWLEDGEMENT_VERSION);
    assert!(profile.validate().is_ok());
}

#[test]
fn persona_v1_deserializes_to_safe_v2_defaults_without_losing_existing_values() {
    let decoded: PersonaSettingsDto = serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "personas": {
            "epsilon": {
                "character_id": "epsilon",
                "name": "Epsilon",
                "first_person_pronoun": "私",
                "user_name": "",
                "user_address": "",
                "relationship": "",
                "speaking_style": "",
                "interests": [],
                "dislikes": [],
                "values": [],
                "background": "",
                "boundaries": [],
                "free_text": "legacy",
                "preset": null,
                "initiative": 9,
                "closeness": 19,
                "humor": 29,
                "response_length": 39,
                "emotional_expression": 49,
                "reaction_interval": 59
            }
        }
    }))
    .expect("v1 persona settings migrate");

    let profile = &decoded.personas["epsilon"];
    assert_eq!(decoded.schema_version, 2);
    assert_eq!(profile.initiative, 9);
    assert_eq!(profile.reaction_interval, 59);
    assert_eq!(profile.free_text, "legacy");
    assert_eq!(profile.machiavellianism, 50);
    assert_eq!(profile.narcissism, 50);
    assert_eq!(profile.psychopathy, 50);
    assert!(!profile.allow_intense_dark_expression);
    assert_eq!(profile.dark_expression_acknowledgement_version, None);
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

#[test]
fn behavior_mode_rules_reject_invalid_schedule_structure() {
    let invalid_rules = [
        ScheduleActivationRuleDto {
            enabled: false,
            mode: CompanionModeDto::Focus,
            days_of_week: Vec::new(),
            start_local_time: "09:00".to_owned(),
            end_local_time: "17:00".to_owned(),
        },
        ScheduleActivationRuleDto {
            enabled: true,
            mode: CompanionModeDto::Focus,
            days_of_week: vec![1, 1],
            start_local_time: "09:00".to_owned(),
            end_local_time: "17:00".to_owned(),
        },
        ScheduleActivationRuleDto {
            enabled: true,
            mode: CompanionModeDto::Focus,
            days_of_week: vec![7],
            start_local_time: "09:00".to_owned(),
            end_local_time: "17:00".to_owned(),
        },
    ];

    for rule in invalid_rules {
        let mut settings = BehaviorSettingsDto::default();
        settings.activation.schedules.push(rule);
        assert!(settings.validate().is_err());
    }

    for (start, end) in [
        ("9:00", "17:00"),
        ("09:00", "17:0"),
        ("24:00", "17:00"),
        ("09:60", "17:00"),
        ("17:00", "17:00"),
    ] {
        let mut settings = BehaviorSettingsDto::default();
        settings
            .activation
            .schedules
            .push(ScheduleActivationRuleDto {
                enabled: false,
                mode: CompanionModeDto::Night,
                days_of_week: vec![0],
                start_local_time: start.to_owned(),
                end_local_time: end.to_owned(),
            });
        assert!(settings.validate().is_err(), "{start}-{end}");
    }

    let mut settings = BehaviorSettingsDto::default();
    settings.activation.schedules = (0..33)
        .map(|_| ScheduleActivationRuleDto {
            enabled: true,
            mode: CompanionModeDto::Normal,
            days_of_week: vec![0],
            start_local_time: "09:00".to_owned(),
            end_local_time: "17:00".to_owned(),
        })
        .collect();
    assert!(settings.validate().is_err());
}

#[test]
fn behavior_mode_rules_reject_invalid_or_duplicate_app_ids() {
    let invalid_app_ids = [
        Vec::new(),
        vec!["   ".to_owned()],
        vec!["bad\0app.exe".to_owned()],
        vec!["bad\u{0007}app.exe".to_owned()],
        vec!["a".repeat(261)],
        vec!["ÄPP.exe".to_owned(), "äpp.EXE".to_owned()],
    ];

    for app_ids in invalid_app_ids {
        let mut settings = BehaviorSettingsDto::default();
        settings.activation.apps.push(AppActivationRuleDto {
            enabled: false,
            mode: CompanionModeDto::Focus,
            app_ids,
        });
        assert!(settings.validate().is_err());
    }

    let mut too_many_app_ids = BehaviorSettingsDto::default();
    too_many_app_ids.activation.apps.push(AppActivationRuleDto {
        enabled: true,
        mode: CompanionModeDto::Focus,
        app_ids: (0..65).map(|index| format!("app-{index}.exe")).collect(),
    });
    assert!(too_many_app_ids.validate().is_err());

    let mut too_many_rules = BehaviorSettingsDto::default();
    too_many_rules.activation.apps = (0..33)
        .map(|index| AppActivationRuleDto {
            enabled: true,
            mode: CompanionModeDto::Normal,
            app_ids: vec![format!("app-{index}.exe")],
        })
        .collect();
    assert!(too_many_rules.validate().is_err());
}

fn valid_schedule_rule() -> ScheduleActivationRuleDto {
    ScheduleActivationRuleDto {
        enabled: true,
        mode: CompanionModeDto::Focus,
        days_of_week: vec![0],
        start_local_time: "09:00".to_owned(),
        end_local_time: "17:00".to_owned(),
    }
}

fn valid_app_rule(app_ids: Vec<String>) -> AppActivationRuleDto {
    AppActivationRuleDto {
        enabled: true,
        mode: CompanionModeDto::Focus,
        app_ids,
    }
}

#[test]
fn behavior_mode_rules_accept_thirty_two_schedules() {
    let mut settings = BehaviorSettingsDto::default();
    settings.activation.schedules = vec![valid_schedule_rule(); 32];
    assert!(settings.validate().is_ok());
}

#[test]
fn behavior_mode_rules_accept_thirty_two_app_rules() {
    let mut settings = BehaviorSettingsDto::default();
    settings.activation.apps = (0..32)
        .map(|index| valid_app_rule(vec![format!("app-{index}.exe")]))
        .collect();
    assert!(settings.validate().is_ok());
}

#[test]
fn behavior_mode_rules_accept_sixty_four_app_ids() {
    let mut settings = BehaviorSettingsDto::default();
    settings.activation.apps.push(valid_app_rule(
        (0..64).map(|index| format!("app-{index}.exe")).collect(),
    ));
    assert!(settings.validate().is_ok());
}

#[test]
fn behavior_mode_rules_accept_two_hundred_sixty_unicode_scalars() {
    let mut settings = BehaviorSettingsDto::default();
    settings
        .activation
        .apps
        .push(valid_app_rule(vec!["界".repeat(260)]));
    assert!(settings.validate().is_ok());
}
