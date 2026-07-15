#![allow(clippy::field_reassign_with_default)]

use parallel_world_desktop::behavior::{ModeResolutionError, ModeResolutionInput, resolve_mode};
use pw_contracts::{
    ActiveModeSourceDto, AppActivationRuleDto, BEHAVIOR_SETTINGS_SCHEMA_VERSION,
    BehaviorSettingsDto, CompanionModeDto, ModeProfileDto, ScheduleActivationRuleDto,
};

fn input(weekday: u8, minutes: u16) -> ModeResolutionInput {
    ModeResolutionInput {
        local_weekday: weekday,
        local_minutes: minutes,
        foreground_app_id: None,
        fullscreen: None,
    }
}

fn schedule(
    mode: CompanionModeDto,
    days_of_week: Vec<u8>,
    start: &str,
    end: &str,
) -> ScheduleActivationRuleDto {
    ScheduleActivationRuleDto {
        enabled: true,
        mode,
        days_of_week,
        start_local_time: start.to_owned(),
        end_local_time: end.to_owned(),
    }
}

#[test]
fn mode_defaults_to_normal_and_returns_the_normal_profile() {
    let settings = BehaviorSettingsDto::default();
    let resolved = resolve_mode(&settings, &input(0, 0)).expect("valid mode input");

    assert_eq!(
        resolved.active_mode.schema_version,
        BEHAVIOR_SETTINGS_SCHEMA_VERSION
    );
    assert_eq!(resolved.active_mode.mode, CompanionModeDto::Normal);
    assert_eq!(resolved.active_mode.source, ActiveModeSourceDto::Default);
    assert_eq!(resolved.active_mode.manual_override, None);
    assert_eq!(resolved.profile, settings.profiles.normal);
}

#[test]
fn mode_manual_override_selects_each_mode_and_preserves_override() {
    for mode in [
        CompanionModeDto::Normal,
        CompanionModeDto::Focus,
        CompanionModeDto::Night,
    ] {
        let mut settings = BehaviorSettingsDto::default();
        settings.manual_mode_override = Some(mode);
        let resolved = resolve_mode(&settings, &input(2, 720)).expect("valid mode input");

        assert_eq!(resolved.active_mode.mode, mode);
        assert_eq!(resolved.active_mode.source, ActiveModeSourceDto::Manual);
        assert_eq!(resolved.active_mode.manual_override, Some(mode));
    }
}

#[test]
fn mode_uses_fixed_cross_tier_precedence() {
    let mut settings = BehaviorSettingsDto::default();
    settings.activation.fullscreen.enabled = true;
    settings.activation.fullscreen.mode = CompanionModeDto::Normal;
    settings.activation.apps.push(AppActivationRuleDto {
        enabled: true,
        mode: CompanionModeDto::Night,
        app_ids: vec!["code.exe".to_owned()],
    });
    settings.activation.schedules.push(schedule(
        CompanionModeDto::Focus,
        vec![0],
        "09:00",
        "17:00",
    ));
    let matching = ModeResolutionInput {
        foreground_app_id: Some("CODE.EXE".to_owned()),
        fullscreen: Some(true),
        ..input(0, 600)
    };

    assert_eq!(
        resolve_mode(&settings, &matching)
            .unwrap()
            .active_mode
            .source,
        ActiveModeSourceDto::Fullscreen
    );
    settings.activation.fullscreen.enabled = false;
    assert_eq!(
        resolve_mode(&settings, &matching)
            .unwrap()
            .active_mode
            .source,
        ActiveModeSourceDto::App
    );
    settings.activation.apps[0].enabled = false;
    assert_eq!(
        resolve_mode(&settings, &matching)
            .unwrap()
            .active_mode
            .source,
        ActiveModeSourceDto::Schedule
    );
    settings.manual_mode_override = Some(CompanionModeDto::Night);
    assert_eq!(
        resolve_mode(&settings, &matching)
            .unwrap()
            .active_mode
            .source,
        ActiveModeSourceDto::Manual
    );
}

#[test]
fn mode_fullscreen_unknown_or_false_does_not_activate() {
    let mut settings = BehaviorSettingsDto::default();
    settings.activation.fullscreen.enabled = true;
    settings.activation.fullscreen.mode = CompanionModeDto::Night;

    for fullscreen in [None, Some(false)] {
        let resolved = resolve_mode(
            &settings,
            &ModeResolutionInput {
                fullscreen,
                ..input(0, 0)
            },
        )
        .unwrap();
        assert_eq!(resolved.active_mode.source, ActiveModeSourceDto::Default);
    }
}

#[test]
fn mode_app_matching_uses_unicode_lowercase_and_quietest_severity() {
    let mut settings = BehaviorSettingsDto::default();
    settings.activation.apps = vec![
        AppActivationRuleDto {
            enabled: true,
            mode: CompanionModeDto::Focus,
            app_ids: vec!["ÄPP.EXE".to_owned()],
        },
        AppActivationRuleDto {
            enabled: true,
            mode: CompanionModeDto::Night,
            app_ids: vec!["äpp.exe".to_owned()],
        },
    ];
    let app_input = ModeResolutionInput {
        foreground_app_id: Some("äPP.Exe".to_owned()),
        ..input(0, 0)
    };

    let forward = resolve_mode(&settings, &app_input).unwrap();
    settings.activation.apps.reverse();
    let reverse = resolve_mode(&settings, &app_input).unwrap();
    assert_eq!(forward.active_mode.mode, CompanionModeDto::Night);
    assert_eq!(forward.active_mode.source, ActiveModeSourceDto::App);
    assert_eq!(reverse.active_mode, forward.active_mode);
}

#[test]
fn mode_same_day_schedule_is_start_inclusive_and_end_exclusive() {
    let mut settings = BehaviorSettingsDto::default();
    settings.activation.schedules.push(schedule(
        CompanionModeDto::Focus,
        vec![2],
        "09:00",
        "17:00",
    ));

    assert_eq!(
        resolve_mode(&settings, &input(2, 540))
            .unwrap()
            .active_mode
            .mode,
        CompanionModeDto::Focus
    );
    assert_eq!(
        resolve_mode(&settings, &input(2, 1_019))
            .unwrap()
            .active_mode
            .mode,
        CompanionModeDto::Focus
    );
    assert_eq!(
        resolve_mode(&settings, &input(2, 1_020))
            .unwrap()
            .active_mode
            .source,
        ActiveModeSourceDto::Default
    );
}

#[test]
fn mode_overnight_schedule_uses_start_day_and_previous_day_at_morning_boundary() {
    let mut settings = BehaviorSettingsDto::default();
    settings.activation.schedules.push(schedule(
        CompanionModeDto::Night,
        vec![4],
        "22:00",
        "02:00",
    ));

    for (weekday, minutes) in [(4, 1_320), (4, 1_439), (5, 0), (5, 119)] {
        assert_eq!(
            resolve_mode(&settings, &input(weekday, minutes))
                .unwrap()
                .active_mode
                .mode,
            CompanionModeDto::Night,
            "weekday={weekday}, minutes={minutes}"
        );
    }
    assert_eq!(
        resolve_mode(&settings, &input(5, 120))
            .unwrap()
            .active_mode
            .source,
        ActiveModeSourceDto::Default
    );
}

#[test]
fn mode_overnight_schedule_wraps_sunday_into_monday() {
    let mut settings = BehaviorSettingsDto::default();
    settings.activation.schedules.push(schedule(
        CompanionModeDto::Focus,
        vec![6],
        "23:00",
        "01:00",
    ));

    assert_eq!(
        resolve_mode(&settings, &input(0, 30))
            .unwrap()
            .active_mode
            .mode,
        CompanionModeDto::Focus
    );
    assert_eq!(
        resolve_mode(&settings, &input(0, 60))
            .unwrap()
            .active_mode
            .source,
        ActiveModeSourceDto::Default
    );
}

#[test]
fn mode_schedule_severity_is_independent_of_rule_order() {
    let mut settings = BehaviorSettingsDto::default();
    settings.activation.schedules = vec![
        schedule(CompanionModeDto::Normal, vec![1], "08:00", "18:00"),
        schedule(CompanionModeDto::Night, vec![1], "09:00", "17:00"),
        schedule(CompanionModeDto::Focus, vec![1], "07:00", "19:00"),
    ];

    let forward = resolve_mode(&settings, &input(1, 600)).unwrap();
    settings.activation.schedules.reverse();
    let reverse = resolve_mode(&settings, &input(1, 600)).unwrap();
    assert_eq!(forward.active_mode.mode, CompanionModeDto::Night);
    assert_eq!(reverse.active_mode, forward.active_mode);
}

#[test]
fn mode_selected_profile_is_cloned_without_modification() {
    let mut settings = BehaviorSettingsDto::default();
    settings.profiles.focus = ModeProfileDto {
        proactive_enabled: true,
        tts_enabled: false,
        character_enabled: true,
        notifications_enabled: true,
        volume: 0.375,
    };
    settings.manual_mode_override = Some(CompanionModeDto::Focus);

    let resolved = resolve_mode(&settings, &input(0, 0)).unwrap();
    assert_eq!(resolved.profile, settings.profiles.focus);
}

#[test]
fn mode_invalid_local_clock_input_returns_stable_errors() {
    let settings = BehaviorSettingsDto::default();
    assert_eq!(
        resolve_mode(&settings, &input(7, 0)).unwrap_err(),
        ModeResolutionError::InvalidWeekday
    );
    assert_eq!(
        resolve_mode(&settings, &input(0, 1_440)).unwrap_err(),
        ModeResolutionError::InvalidLocalMinutes
    );
    assert_eq!(
        ModeResolutionError::InvalidWeekday.to_string(),
        "local weekday must be between 0 and 6"
    );
    assert_eq!(
        ModeResolutionError::InvalidLocalMinutes.to_string(),
        "local minutes must be between 0 and 1439"
    );
}
