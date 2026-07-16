use pw_contracts::{ChatPlacementDto, ThemePreferenceDto, UiPreferencesDto};
use pw_platform::paths::AppDataLayout;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

use crate::ui::{load_preferences, save_preferences};

pub const UI_PREFERENCES_EVENT: &str = "ui-preferences-changed";

trait ChatPlacementWindows {
    fn show_chat(&mut self) -> Result<(), String>;
    fn hide_chat(&mut self) -> Result<(), String>;
    fn show_settings(&mut self) -> Result<(), String>;
}

fn apply_chat_placement(
    current: UiPreferencesDto,
    placement: ChatPlacementDto,
    windows: &mut impl ChatPlacementWindows,
    persist: impl FnOnce(&UiPreferencesDto) -> Result<(), String>,
) -> Result<UiPreferencesDto, String> {
    if current.chat_placement == placement {
        if placement == ChatPlacementDto::Popped {
            windows.show_chat()?;
        }
        return Ok(current);
    }
    let mut next = current;
    next.chat_placement = placement;
    match placement {
        ChatPlacementDto::Popped => {
            windows.show_chat()?;
            if let Err(error) = persist(&next) {
                let _ = windows.hide_chat();
                return Err(error);
            }
        }
        ChatPlacementDto::Docked => {
            windows.show_settings()?;
            windows.hide_chat()?;
            if let Err(error) = persist(&next) {
                let _ = windows.show_chat();
                return Err(error);
            }
        }
    }
    Ok(next)
}

struct TauriChatPlacementWindows<R: Runtime>(AppHandle<R>);

impl<R: Runtime> ChatPlacementWindows for TauriChatPlacementWindows<R> {
    fn show_chat(&mut self) -> Result<(), String> {
        let window = self
            .0
            .get_webview_window("chat")
            .ok_or_else(|| "chat window is not available".to_owned())?;
        window.show().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())
    }

    fn hide_chat(&mut self) -> Result<(), String> {
        self.0
            .get_webview_window("chat")
            .ok_or_else(|| "chat window is not available".to_owned())?
            .hide()
            .map_err(|error| error.to_string())
    }

    fn show_settings(&mut self) -> Result<(), String> {
        let window = self
            .0
            .get_webview_window("settings")
            .ok_or_else(|| "settings window is not available".to_owned())?;
        window.show().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())
    }
}

fn emit_preferences<R: Runtime>(app: &AppHandle<R>, value: &UiPreferencesDto) {
    if let Err(error) = app.emit(UI_PREFERENCES_EVENT, value) {
        tracing::warn!(%error, "failed to emit UI preferences");
    }
}

#[tauri::command]
#[must_use]
#[allow(clippy::needless_pass_by_value)] // Tauri command extraction owns State.
pub fn get_ui_preferences(layout: State<'_, AppDataLayout>) -> UiPreferencesDto {
    load_preferences(&layout)
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri command extraction owns arguments.
/// Persists a theme preference and broadcasts the resulting UI state.
///
/// # Errors
///
/// Returns an error when the preference cannot be persisted.
pub fn set_theme_preference<R: Runtime>(
    app: AppHandle<R>,
    layout: State<'_, AppDataLayout>,
    theme: ThemePreferenceDto,
) -> Result<UiPreferencesDto, String> {
    let mut preferences = load_preferences(&layout);
    preferences.theme = theme;
    save_preferences(&layout, &preferences)?;
    emit_preferences(&app, &preferences);
    Ok(preferences)
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri command extraction owns arguments.
/// Moves conversation UI between the management and chat windows.
///
/// # Errors
///
/// Returns an error when a window transition or preference write fails.
pub fn set_chat_placement<R: Runtime>(
    app: AppHandle<R>,
    layout: State<'_, AppDataLayout>,
    placement: ChatPlacementDto,
) -> Result<UiPreferencesDto, String> {
    let current = load_preferences(&layout);
    let mut windows = TauriChatPlacementWindows(app.clone());
    let next = apply_chat_placement(current, placement, &mut windows, |value| {
        save_preferences(&layout, value)
    })?;
    emit_preferences(&app, &next);
    Ok(next)
}

/// Restores the persisted chat window visibility during startup.
///
/// # Errors
///
/// Returns an error when the chat window is unavailable or cannot be changed.
pub fn restore_chat_placement<R: Runtime>(
    app: &AppHandle<R>,
    layout: &AppDataLayout,
) -> Result<(), String> {
    let preferences = load_preferences(layout);
    let chat = app
        .get_webview_window("chat")
        .ok_or_else(|| "chat window is not available".to_owned())?;
    match preferences.chat_placement {
        ChatPlacementDto::Docked => chat.hide(),
        ChatPlacementDto::Popped => chat.show(),
    }
    .map_err(|error| error.to_string())
}

/// Converts a native chat-window close request into a dock operation.
///
/// # Errors
///
/// Returns an error when a window transition or preference write fails.
pub fn dock_chat_on_close<R: Runtime>(
    app: &AppHandle<R>,
    layout: &AppDataLayout,
) -> Result<(), String> {
    let current = load_preferences(layout);
    if current.chat_placement == ChatPlacementDto::Docked {
        return Ok(());
    }
    let mut windows = TauriChatPlacementWindows(app.clone());
    let next = apply_chat_placement(current, ChatPlacementDto::Docked, &mut windows, |value| {
        save_preferences(layout, value)
    })?;
    emit_preferences(app, &next);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ChatPlacementWindows, apply_chat_placement};
    use pw_contracts::{ChatPlacementDto, UiPreferencesDto};

    #[derive(Default)]
    struct Windows {
        actions: Vec<&'static str>,
    }

    impl ChatPlacementWindows for Windows {
        fn show_chat(&mut self) -> Result<(), String> {
            self.actions.push("show_chat");
            Ok(())
        }
        fn hide_chat(&mut self) -> Result<(), String> {
            self.actions.push("hide_chat");
            Ok(())
        }
        fn show_settings(&mut self) -> Result<(), String> {
            self.actions.push("show_settings");
            Ok(())
        }
    }

    #[test]
    fn popout_save_failure_rolls_back_to_docked_visibility() {
        let mut windows = Windows::default();
        let result = apply_chat_placement(
            UiPreferencesDto::default(),
            ChatPlacementDto::Popped,
            &mut windows,
            |_| Err("disk full".into()),
        );
        assert_eq!(result.unwrap_err(), "disk full");
        assert_eq!(windows.actions, ["show_chat", "hide_chat"]);
    }

    #[test]
    fn dock_save_failure_restores_the_popout() {
        let current = UiPreferencesDto {
            chat_placement: ChatPlacementDto::Popped,
            ..UiPreferencesDto::default()
        };
        let mut windows = Windows::default();
        let result = apply_chat_placement(current, ChatPlacementDto::Docked, &mut windows, |_| {
            Err("read only".into())
        });
        assert_eq!(result.unwrap_err(), "read only");
        assert_eq!(windows.actions, ["show_settings", "hide_chat", "show_chat"]);
    }

    #[test]
    fn requesting_the_existing_popout_refocuses_chat_without_persisting() {
        let current = UiPreferencesDto {
            chat_placement: ChatPlacementDto::Popped,
            ..UiPreferencesDto::default()
        };
        let mut windows = Windows::default();
        let mut persisted = false;
        let result = apply_chat_placement(
            current.clone(),
            ChatPlacementDto::Popped,
            &mut windows,
            |_| {
                persisted = true;
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(result, current);
        assert_eq!(windows.actions, ["show_chat"]);
        assert!(!persisted);
    }
}
