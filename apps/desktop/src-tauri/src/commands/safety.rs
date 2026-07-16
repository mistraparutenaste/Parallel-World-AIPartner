//! Settings-only commands and pre-model safeword interception.

use pw_contracts::{
    DARK_EXPRESSION_SAFETY_CHANGED_EVENT, DARK_EXPRESSION_SAFETY_SCHEMA_VERSION,
    DarkExpressionSafetyChangedEventDto, DarkExpressionSafetySettingsDto, SAFEWORD_TRIGGERED_EVENT,
    SafewordTriggeredEventDto,
};
use pw_platform::paths::AppDataLayout;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

use crate::behavior::{
    DarkExpressionSafetyState, load_dark_expression_safety_checked, safe_word_matches,
    sanitize_safe_word, save_dark_expression_safety,
};

fn effective_settings(
    layout: &AppDataLayout,
    state: &DarkExpressionSafetyState,
) -> Result<DarkExpressionSafetySettingsDto, String> {
    let mut settings =
        load_dark_expression_safety_checked(layout).map_err(|error| error.to_string())?;
    settings.dark_expression_paused = state.is_paused();
    Ok(settings)
}

fn emit_changed<R: Runtime>(app: &AppHandle<R>, settings: DarkExpressionSafetySettingsDto) {
    if let Err(error) = app.emit(
        DARK_EXPRESSION_SAFETY_CHANGED_EVENT,
        DarkExpressionSafetyChangedEventDto {
            schema_version: DARK_EXPRESSION_SAFETY_SCHEMA_VERSION,
            settings,
        },
    ) {
        tracing::warn!(%error, "failed to emit dark expression safety settings");
    }
}

pub(crate) fn set_safe_word_for_layout(
    layout: &AppDataLayout,
    state: &DarkExpressionSafetyState,
    safe_word: Option<String>,
) -> Result<DarkExpressionSafetySettingsDto, String> {
    let safe_word = sanitize_safe_word(safe_word)?;
    let settings = DarkExpressionSafetySettingsDto {
        safe_word,
        dark_expression_paused: state.is_paused(),
        ..DarkExpressionSafetySettingsDto::default()
    };
    save_dark_expression_safety(layout, &settings)?;
    Ok(settings)
}

pub(crate) fn resume_dark_expression_for_layout(
    layout: &AppDataLayout,
    state: &DarkExpressionSafetyState,
) -> Result<DarkExpressionSafetySettingsDto, String> {
    let mut settings =
        load_dark_expression_safety_checked(layout).map_err(|error| error.to_string())?;
    settings.dark_expression_paused = false;
    save_dark_expression_safety(layout, &settings)?;
    state.set_paused(false);
    Ok(settings)
}

/// Returns true when the complete input was consumed as the configured safeword.
///
/// Invalid safety storage pauses intense expression but does not block ordinary chat.
pub(crate) fn intercept_user_input<R: Runtime>(app: &AppHandle<R>, text: &str) -> bool {
    let layout = app.state::<AppDataLayout>();
    let state = app.state::<DarkExpressionSafetyState>();
    let Ok(mut settings) = load_dark_expression_safety_checked(&layout) else {
        state.pause();
        return false;
    };
    if !safe_word_matches(settings.safe_word.as_deref(), text) {
        return false;
    }

    state.pause();
    app.state::<crate::chat::ChatService>().cancel();
    app.state::<crate::tts::TtsService>().stop(app);
    settings.dark_expression_paused = true;
    let pause_persisted = save_dark_expression_safety(&layout, &settings).is_ok();
    emit_changed(app, settings);
    let _ = app.emit(
        SAFEWORD_TRIGGERED_EVENT,
        SafewordTriggeredEventDto {
            schema_version: DARK_EXPRESSION_SAFETY_SCHEMA_VERSION,
            pause_persisted,
        },
    );
    tracing::info!("safeword-triggered");
    true
}

/// Loads the user-wide safeword and effective process-local pause state.
///
/// # Errors
///
/// Returns a stable read or validation error without exposing the phrase.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn get_dark_expression_safety_settings(
    layout: State<'_, AppDataLayout>,
    state: State<'_, DarkExpressionSafetyState>,
) -> Result<DarkExpressionSafetySettingsDto, String> {
    effective_settings(&layout, &state)
}

/// Replaces or clears the user-wide safeword without changing the pause latch.
///
/// # Errors
///
/// Returns a validation or persistence error without exposing the phrase.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn set_safe_word<R: Runtime>(
    app: AppHandle<R>,
    layout: State<'_, AppDataLayout>,
    state: State<'_, DarkExpressionSafetyState>,
    safe_word: Option<String>,
) -> Result<DarkExpressionSafetySettingsDto, String> {
    let settings = set_safe_word_for_layout(&layout, &state, safe_word)?;
    emit_changed(&app, settings.clone());
    Ok(settings)
}

/// Explicitly resumes intense dark expression without generating a reply.
///
/// # Errors
///
/// Returns a read or persistence error and leaves the process latch paused.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn resume_dark_expression<R: Runtime>(
    app: AppHandle<R>,
    layout: State<'_, AppDataLayout>,
    state: State<'_, DarkExpressionSafetyState>,
) -> Result<DarkExpressionSafetySettingsDto, String> {
    let settings = resume_dark_expression_for_layout(&layout, &state)?;
    emit_changed(&app, settings.clone());
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use pw_platform::paths::AppDataLayout;

    use super::{resume_dark_expression_for_layout, set_safe_word_for_layout};
    use crate::behavior::{DarkExpressionSafetyState, load_dark_expression_safety_checked};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestLayout {
        layout: AppDataLayout,
    }

    impl TestLayout {
        fn new(name: &str) -> Self {
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "pw-safety-command-{name}-{}-{sequence}",
                std::process::id()
            ));
            let layout = AppDataLayout::under(root);
            layout.create_all().unwrap();
            Self { layout }
        }
    }

    impl Drop for TestLayout {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.layout.root);
        }
    }

    #[test]
    fn setting_a_word_preserves_the_process_pause_latch() {
        let test = TestLayout::new("set");
        let state = DarkExpressionSafetyState::default();
        state.pause();

        let saved =
            set_safe_word_for_layout(&test.layout, &state, Some(" stop ".to_owned())).unwrap();

        assert_eq!(saved.safe_word.as_deref(), Some("stop"));
        assert!(saved.dark_expression_paused);
        assert_eq!(
            load_dark_expression_safety_checked(&test.layout).unwrap(),
            saved
        );
    }

    #[test]
    fn resume_updates_disk_before_releasing_the_process_latch() {
        let test = TestLayout::new("resume");
        let state = DarkExpressionSafetyState::default();
        state.pause();
        set_safe_word_for_layout(&test.layout, &state, Some("stop".to_owned())).unwrap();

        let saved = resume_dark_expression_for_layout(&test.layout, &state).unwrap();

        assert!(!saved.dark_expression_paused);
        assert!(!state.is_paused());
    }

    #[test]
    fn resume_failure_keeps_the_process_latch_paused() {
        let test = TestLayout::new("resume-failure");
        let state = DarkExpressionSafetyState::default();
        state.pause();
        fs::write(
            test.layout.config.join("dark-expression-safety.json"),
            b"{invalid-private",
        )
        .unwrap();

        assert!(resume_dark_expression_for_layout(&test.layout, &state).is_err());
        assert!(state.is_paused());
    }
}
