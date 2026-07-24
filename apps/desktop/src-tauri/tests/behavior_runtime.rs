use parallel_world_desktop::behavior::{
    BehaviorRuntimeSnapshot, RuntimeCollectionHealth, RuntimeMode, resolve_runtime_snapshot,
};
use pw_contracts::{BehaviorSettingsDto, CompanionModeDto};

#[test]
fn runtime_snapshot_uses_manual_mode_and_preserves_collector_health() {
    let settings = BehaviorSettingsDto {
        manual_mode_override: Some(CompanionModeDto::Focus),
        ..Default::default()
    };

    let snapshot = resolve_runtime_snapshot(
        &settings,
        2,
        9 * 60,
        Some("code.exe".to_owned()),
        Some(false),
        RuntimeCollectionHealth::Healthy {
            last_activity_at: Some(123),
        },
    )
    .expect("valid runtime snapshot");

    assert_eq!(
        snapshot,
        BehaviorRuntimeSnapshot {
            mode: RuntimeMode::Focus,
            proactive_enabled: false,
            tts_enabled: false,
            collection: RuntimeCollectionHealth::Healthy {
                last_activity_at: Some(123),
            },
        }
    );
}

#[test]
fn runtime_snapshot_fails_closed_for_invalid_local_time() {
    let settings = BehaviorSettingsDto::default();
    assert!(
        resolve_runtime_snapshot(
            &settings,
            7,
            0,
            None,
            None,
            RuntimeCollectionHealth::Disabled,
        )
        .is_err()
    );
}
