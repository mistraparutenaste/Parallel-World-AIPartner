use parallel_world_desktop::desktop_controls::{
    DesktopControlAction, ShortcutBindings, TraySettingsAction, apply_tray_settings_action,
    cycle_mode,
};
use pw_contracts::{BehaviorSettingsDto, CompanionModeDto, ConsentStateDto, ShortcutSettingsDto};

fn shortcuts() -> ShortcutSettingsDto {
    ShortcutSettingsDto {
        push_to_talk: "Ctrl+Alt+Space".to_owned(),
        toggle_mute: "Ctrl+Alt+M".to_owned(),
        open_control_center: "Ctrl+Alt+P".to_owned(),
        toggle_character: "Ctrl+Alt+C".to_owned(),
        cycle_mode: "Ctrl+Alt+F".to_owned(),
    }
}

#[test]
fn shortcut_bindings_require_five_unique_non_empty_values() {
    let bindings = ShortcutBindings::from_settings(&shortcuts()).unwrap();

    assert_eq!(bindings.len(), 5);
    assert_eq!(
        bindings.action_for("ctrl+alt+p"),
        Some(DesktopControlAction::OpenControlCenter)
    );

    let mut duplicate = shortcuts();
    duplicate.cycle_mode = duplicate.open_control_center.clone();
    assert_eq!(
        ShortcutBindings::from_settings(&duplicate).unwrap_err(),
        "desktop shortcuts must be unique"
    );

    let mut empty = shortcuts();
    empty.toggle_mute.clear();
    assert_eq!(
        ShortcutBindings::from_settings(&empty).unwrap_err(),
        "desktop shortcuts must not be empty"
    );
}

#[test]
fn cycle_mode_moves_through_all_modes_and_back_to_automatic() {
    assert_eq!(cycle_mode(None), Some(CompanionModeDto::Normal));
    assert_eq!(
        cycle_mode(Some(CompanionModeDto::Normal)),
        Some(CompanionModeDto::Focus)
    );
    assert_eq!(
        cycle_mode(Some(CompanionModeDto::Focus)),
        Some(CompanionModeDto::Night)
    );
    assert_eq!(cycle_mode(Some(CompanionModeDto::Night)), None);
}

#[test]
fn tray_collection_toggle_never_grants_consent_and_snooze_is_bounded() {
    let pending = BehaviorSettingsDto::default();
    let still_disabled =
        apply_tray_settings_action(&pending, TraySettingsAction::ToggleCollection, 1_000);
    assert!(!still_disabled.collection_enabled);
    assert_eq!(still_disabled.consent, ConsentStateDto::Pending);

    let accepted = BehaviorSettingsDto {
        consent: ConsentStateDto::Accepted,
        ..BehaviorSettingsDto::default()
    };
    let enabled =
        apply_tray_settings_action(&accepted, TraySettingsAction::ToggleCollection, 1_000);
    assert!(enabled.collection_enabled);

    let snoozed = apply_tray_settings_action(&accepted, TraySettingsAction::SnoozeOneHour, 1_000);
    assert_eq!(snoozed.proactive_snoozed_until, Some(4_600));
}
