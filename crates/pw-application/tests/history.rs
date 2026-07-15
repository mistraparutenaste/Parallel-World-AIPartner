use pw_application::history::{
    PersistedProactiveAssistantMessage, ProactiveAssistantHistory, ProactiveAssistantHistoryError,
    ProactiveAssistantMessage,
};

struct ProactiveOnlyHistory;

impl ProactiveAssistantHistory for ProactiveOnlyHistory {
    fn append_proactive_assistant(
        &mut self,
        _message: &ProactiveAssistantMessage,
    ) -> Result<PersistedProactiveAssistantMessage, ProactiveAssistantHistoryError> {
        Ok(PersistedProactiveAssistantMessage {
            turn_id: 1,
            message_id: 2,
        })
    }
}

#[test]
fn proactive_assistant_history_is_a_separate_minimal_port() {
    let mut history = ProactiveOnlyHistory;
    let persisted = history
        .append_proactive_assistant(&ProactiveAssistantMessage {
            conversation_id: "chat".into(),
            content: "hello".into(),
            created_at: 1,
        })
        .unwrap();
    assert_eq!(persisted.turn_id, 1);
    assert_eq!(persisted.message_id, 2);
}

#[test]
fn proactive_assistant_history_error_is_opaque() {
    let error = ProactiveAssistantHistoryError;
    assert_eq!(
        format!("{error}"),
        "proactive assistant history unavailable"
    );
    assert_eq!(format!("{error:?}"), "ProactiveAssistantHistoryError");
    assert!(std::error::Error::source(&error).is_none());
}
