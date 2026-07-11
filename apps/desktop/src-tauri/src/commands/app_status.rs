use pw_contracts::dto::{AppStatusDto, ConversationStateDto, SCHEMA_VERSION};

#[tauri::command]
pub fn get_app_status() -> AppStatusDto {
    AppStatusDto {
        schema_version: SCHEMA_VERSION,
        conversation_state: ConversationStateDto::Idle,
    }
}

#[cfg(test)]
mod tests {
    use super::get_app_status;
    use pw_contracts::dto::{ConversationStateDto, SCHEMA_VERSION};

    #[test]
    fn returns_versioned_idle_status() {
        let status = get_app_status();
        assert_eq!(status.schema_version, SCHEMA_VERSION);
        assert_eq!(status.conversation_state, ConversationStateDto::Idle);
    }
}
