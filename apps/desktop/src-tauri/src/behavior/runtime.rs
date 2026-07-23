//! Pure runtime snapshot composition shared by the background service and IPC.

use pw_contracts::{BehaviorSettingsDto, CompanionModeDto};

use super::{ModeResolutionError, ModeResolutionInput, resolve_mode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMode {
    Normal,
    Focus,
    Night,
}

impl From<CompanionModeDto> for RuntimeMode {
    fn from(value: CompanionModeDto) -> Self {
        match value {
            CompanionModeDto::Normal => Self::Normal,
            CompanionModeDto::Focus => Self::Focus,
            CompanionModeDto::Night => Self::Night,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeCollectionHealth {
    Disabled,
    Healthy { last_activity_at: Option<i64> },
    Degraded { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BehaviorRuntimeSnapshot {
    pub mode: RuntimeMode,
    pub proactive_enabled: bool,
    pub tts_enabled: bool,
    pub collection: RuntimeCollectionHealth,
}

/// Combines validated settings, local context, and collector health into the
/// immutable state consumed by the proactive worker and status IPC.
///
/// # Errors
///
/// Returns an error for invalid local weekday/minute input.
pub fn resolve_runtime_snapshot(
    settings: &BehaviorSettingsDto,
    local_weekday: u8,
    local_minutes: u16,
    foreground_app_id: Option<String>,
    fullscreen: Option<bool>,
    collection: RuntimeCollectionHealth,
) -> Result<BehaviorRuntimeSnapshot, ModeResolutionError> {
    let resolved = resolve_mode(
        settings,
        &ModeResolutionInput {
            local_weekday,
            local_minutes,
            foreground_app_id,
            fullscreen,
        },
    )?;
    Ok(BehaviorRuntimeSnapshot {
        mode: resolved.active_mode.mode.into(),
        proactive_enabled: resolved.profile.proactive_enabled,
        tts_enabled: resolved.profile.tts_enabled,
        collection,
    })
}
