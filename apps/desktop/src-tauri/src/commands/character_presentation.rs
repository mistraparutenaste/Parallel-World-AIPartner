use pw_contracts::dto::{CHARACTER_PRESENTATION_SCHEMA_VERSION, CharacterPresentationSettingsDto};
use pw_domain::character_presentation::{
    CharacterPresentationSettings, CharacterPresentationValidationError,
};
use serde::Serialize;
use std::sync::{Mutex, RwLock};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

pub const CHARACTER_PRESENTATION_CHANGED_EVENT: &str = "character-presentation://changed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CharacterPresentationCommandError {
    UnsupportedSchema,
    InvalidModel,
    InvalidExpression,
    InvalidMotion,
    StateUnavailable,
    CharacterWindowUnavailable,
    ClickThroughUnavailable,
}

pub struct CharacterPresentationState {
    transaction: Mutex<()>,
    current: RwLock<CharacterPresentationSettingsDto>,
}
impl Default for CharacterPresentationState {
    fn default() -> Self {
        Self {
            transaction: Mutex::new(()),
            current: RwLock::new(CharacterPresentationSettingsDto {
                schema_version: CHARACTER_PRESENTATION_SCHEMA_VERSION,
                revision: 0,
                model_id: "mark".into(),
                expression_id: String::new(),
                motion_group: "Idle".into(),
                motion_index: 0,
                click_through: false,
            }),
        }
    }
}
impl CharacterPresentationState {
    pub fn current(&self) -> CharacterPresentationSettingsDto {
        self.current
            .read()
            .expect("character presentation state mutex poisoned")
            .clone()
    }
    fn validate(
        value: &CharacterPresentationSettingsDto,
    ) -> Result<(), CharacterPresentationCommandError> {
        if value.schema_version != CHARACTER_PRESENTATION_SCHEMA_VERSION {
            return Err(CharacterPresentationCommandError::UnsupportedSchema);
        }
        CharacterPresentationSettings::try_new(
            &value.model_id,
            &value.expression_id,
            &value.motion_group,
            value.motion_index,
            value.click_through,
        )
        .map(|_| ())
        .map_err(|error| match error {
            CharacterPresentationValidationError::UnknownModel => {
                CharacterPresentationCommandError::InvalidModel
            }
            CharacterPresentationValidationError::UnknownExpression => {
                CharacterPresentationCommandError::InvalidExpression
            }
            CharacterPresentationValidationError::UnknownMotion => {
                CharacterPresentationCommandError::InvalidMotion
            }
        })
    }
    fn commit_with<M, E>(
        &self,
        mut value: CharacterPresentationSettingsDto,
        mutate_window: M,
        emit: E,
    ) -> Result<(CharacterPresentationSettingsDto, bool), CharacterPresentationCommandError>
    where
        M: FnOnce(bool) -> Result<(), CharacterPresentationCommandError>,
        E: FnOnce(&CharacterPresentationSettingsDto) -> Result<(), ()>,
    {
        Self::validate(&value)?;
        let _transaction = self.transaction.lock().map_err(|_| CharacterPresentationCommandError::StateUnavailable)?;
        let revision = self.current.read().map_err(|_| CharacterPresentationCommandError::StateUnavailable)?.revision.checked_add(1).ok_or(CharacterPresentationCommandError::StateUnavailable)?;
        value.revision = revision;
        mutate_window(value.click_through)?;
        *self.current.write().map_err(|_| CharacterPresentationCommandError::StateUnavailable)? = value.clone();
        let event_delivery_failed = emit(&value).is_err();
        Ok((value, event_delivery_failed))
    }
}

#[tauri::command]
pub fn get_character_presentation(
    state: State<'_, CharacterPresentationState>,
) -> CharacterPresentationSettingsDto {
    state.current()
}

#[tauri::command]
pub fn set_character_presentation<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, CharacterPresentationState>,
    value: CharacterPresentationSettingsDto,
) -> Result<CharacterPresentationSettingsDto, CharacterPresentationCommandError> {
    let character = app
        .get_webview_window("character")
        .ok_or(CharacterPresentationCommandError::CharacterWindowUnavailable)?;
    let (value, event_delivery_failed) = state.commit_with(
        value,
        |click_through| character.set_ignore_cursor_events(click_through).map_err(|_| CharacterPresentationCommandError::ClickThroughUnavailable),
        |committed| app.emit(CHARACTER_PRESENTATION_CHANGED_EVENT, committed).map_err(|_| ()),
    )?;
    if event_delivery_failed { tracing::warn!(revision = value.revision, "character presentation committed but event delivery failed"); }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn dto() -> CharacterPresentationSettingsDto {
        CharacterPresentationSettingsDto {
            schema_version: 1,
            revision: 0,
            model_id: "epsilon-free".into(),
            expression_id: "Smile".into(),
            motion_group: "Tap".into(),
            motion_index: 0,
            click_through: false,
        }
    }
    #[test]
    fn state_rejects_stale_schema_and_invalid_catalog_values() {
        let state = CharacterPresentationState::default();
        let mut stale = dto();
        stale.schema_version = 0;
        assert_eq!(
            state.commit_with(stale, |_| Ok(()), |_| Ok(())).map(|value| value.0),
            Err(CharacterPresentationCommandError::UnsupportedSchema)
        );
        let mut invalid = dto();
        invalid.expression_id = "missing".into();
        assert_eq!(
            state.commit_with(invalid, |_| Ok(()), |_| Ok(())).map(|value| value.0),
            Err(CharacterPresentationCommandError::InvalidExpression)
        );
    }
    #[test]
    fn state_replaces_and_returns_valid_settings() {
        let state = CharacterPresentationState::default();
        let value = dto();
        let committed = state.commit_with(value, |_| Ok(()), |_| Ok(())).unwrap().0;
        assert_eq!(committed.revision, 1);
        assert_eq!(state.current(), committed);
    }

    #[test]
    fn window_failure_leaves_state_and_event_unchanged() {
        let state = CharacterPresentationState::default(); let before = state.current(); let mut emitted = false;
        let result = state.commit_with(dto(), |_| Err(CharacterPresentationCommandError::ClickThroughUnavailable), |_| { emitted = true; Ok(()) });
        assert_eq!(result, Err(CharacterPresentationCommandError::ClickThroughUnavailable));
        assert_eq!(state.current(), before); assert!(!emitted);
    }

    #[test]
    fn event_failure_is_a_committed_success_with_diagnostic_flag() {
        let state = CharacterPresentationState::default();
        let (committed, failed) = state.commit_with(dto(), |_| Ok(()), |_| Err(())).unwrap();
        assert!(failed); assert_eq!(state.current(), committed); assert_eq!(committed.revision, 1);
    }

    #[test]
    fn concurrent_sets_keep_window_state_revision_and_event_order_consistent() {
        use std::{sync::{Arc, mpsc}, thread, time::Duration};
        let state = Arc::new(CharacterPresentationState::default());
        let window = Arc::new(Mutex::new(false));
        let events = Arc::new(Mutex::new(Vec::new()));
        let (entered_tx, entered_rx) = mpsc::channel(); let (release_tx, release_rx) = mpsc::channel();
        let a_state = Arc::clone(&state); let a_window = Arc::clone(&window); let a_events = Arc::clone(&events);
        let mut a = dto(); a.click_through = true;
        let first = thread::spawn(move || a_state.commit_with(a, |value| { entered_tx.send(()).unwrap(); release_rx.recv().unwrap(); *a_window.lock().unwrap() = value; Ok(()) }, |value| { a_events.lock().unwrap().push(value.clone()); Ok(()) }).unwrap());
        entered_rx.recv().unwrap();
        let b_state = Arc::clone(&state); let b_window = Arc::clone(&window); let b_events = Arc::clone(&events);
        let mut b = dto(); b.click_through = false; b.expression_id = "Angry".into();
        let second = thread::spawn(move || b_state.commit_with(b, |value| { *b_window.lock().unwrap() = value; Ok(()) }, |value| { b_events.lock().unwrap().push(value.clone()); Ok(()) }).unwrap());
        thread::sleep(Duration::from_millis(20)); release_tx.send(()).unwrap();
        let first = first.join().unwrap().0; let second = second.join().unwrap().0;
        assert_eq!([first.revision, second.revision], [1, 2]);
        assert_eq!(*window.lock().unwrap(), state.current().click_through);
        let events = events.lock().unwrap(); assert_eq!(events.iter().map(|value| value.revision).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(events.last(), Some(&state.current()));
    }
}
