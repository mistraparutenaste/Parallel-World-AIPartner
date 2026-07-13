//! Signed application updater contracts.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Stable updater lifecycle exposed to the Settings window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "UpdateStatusDto.ts")]
pub enum UpdateStatusDto {
    Disabled,
    Checking,
    UpToDate,
    Available,
    Downloading,
    Installing,
    RestartPending,
    Failed,
}

/// Current updater state. All fields are safe to display to the user.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "UpdateStateDto.ts")]
pub struct UpdateStateDto {
    pub schema_version: u16,
    pub status: UpdateStatusDto,
    pub current_version: String,
    pub available_version: Option<String>,
    pub notes: Option<String>,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{UpdateStateDto, UpdateStatusDto};

    #[test]
    fn update_state_round_trips_with_snake_case_status() {
        let state = UpdateStateDto {
            schema_version: 1,
            status: UpdateStatusDto::RestartPending,
            current_version: "1.0.0".into(),
            available_version: Some("1.1.0".into()),
            notes: Some("Security update".into()),
            error: None,
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("restart_pending"));
        assert_eq!(
            serde_json::from_str::<UpdateStateDto>(&json).unwrap(),
            state
        );
    }
}
