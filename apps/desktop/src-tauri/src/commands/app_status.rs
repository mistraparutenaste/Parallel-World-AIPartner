//! Application status query command.

use pw_contracts::{AppStatusDto, ConversationStateDto, SCHEMA_VERSION};

/// Pure snapshot builder; kept separate from the Tauri macro wrapper
/// so it can be unit-tested without a running app.
fn current_app_status() -> AppStatusDto {
    AppStatusDto {
        schema_version: SCHEMA_VERSION,
        conversation_state: ConversationStateDto::Idle,
    }
}

/// Returns the current application status to an authorized webview.
#[tauri::command]
#[must_use]
pub fn get_app_status() -> AppStatusDto {
    current_app_status()
}

#[cfg(test)]
mod tests {
    #[test]
    fn reports_idle_status_with_current_schema_version() {
        let status = super::current_app_status();
        assert_eq!(status.schema_version, pw_contracts::SCHEMA_VERSION);
        assert_eq!(
            status.conversation_state,
            pw_contracts::ConversationStateDto::Idle
        );
    }
}
