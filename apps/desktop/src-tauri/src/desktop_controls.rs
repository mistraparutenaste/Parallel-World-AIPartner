//! Desktop tray and global-shortcut control policy.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

use pw_contracts::{
    BEHAVIOR_SETTINGS_CHANGED_EVENT, BEHAVIOR_SETTINGS_SCHEMA_VERSION,
    BehaviorSettingsChangedEventDto, BehaviorSettingsDto, CompanionModeDto, ConsentStateDto,
    ShortcutSettingsDto,
};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::behavior::{load_behavior_settings_checked, save_behavior_settings};
use crate::speech::SpeechService;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopControlAction {
    PushToTalk,
    ToggleMute,
    OpenControlCenter,
    ToggleCharacter,
    CycleMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutBindings {
    by_normalized_shortcut: HashMap<String, DesktopControlAction>,
}

impl ShortcutBindings {
    /// Builds the complete desktop shortcut map.
    ///
    /// # Errors
    ///
    /// Returns an error when any shortcut is empty or two actions use the same
    /// normalized shortcut.
    pub fn from_settings(settings: &ShortcutSettingsDto) -> Result<Self, &'static str> {
        let entries = [
            (&settings.push_to_talk, DesktopControlAction::PushToTalk),
            (&settings.toggle_mute, DesktopControlAction::ToggleMute),
            (
                &settings.open_control_center,
                DesktopControlAction::OpenControlCenter,
            ),
            (
                &settings.toggle_character,
                DesktopControlAction::ToggleCharacter,
            ),
            (&settings.cycle_mode, DesktopControlAction::CycleMode),
        ];
        let mut seen = HashSet::with_capacity(entries.len());
        let mut by_normalized_shortcut = HashMap::with_capacity(entries.len());
        for (shortcut, action) in entries {
            let normalized = normalize_shortcut(shortcut);
            if normalized.is_empty() {
                return Err("desktop shortcuts must not be empty");
            }
            if !seen.insert(normalized.clone()) {
                return Err("desktop shortcuts must be unique");
            }
            by_normalized_shortcut.insert(normalized, action);
        }
        Ok(Self {
            by_normalized_shortcut,
        })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_normalized_shortcut.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_normalized_shortcut.is_empty()
    }

    #[must_use]
    pub fn action_for(&self, shortcut: &str) -> Option<DesktopControlAction> {
        self.by_normalized_shortcut
            .get(&normalize_shortcut(shortcut))
            .copied()
    }
}

fn normalize_shortcut(value: &str) -> String {
    value
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join("+")
}

#[must_use]
pub const fn cycle_mode(mode: Option<CompanionModeDto>) -> Option<CompanionModeDto> {
    match mode {
        None => Some(CompanionModeDto::Normal),
        Some(CompanionModeDto::Normal) => Some(CompanionModeDto::Focus),
        Some(CompanionModeDto::Focus) => Some(CompanionModeDto::Night),
        Some(CompanionModeDto::Night) => None,
    }
}

#[derive(Debug, Default)]
pub struct DesktopControlState {
    capture_muted: AtomicBool,
}

impl DesktopControlState {
    fn toggle_capture_mute(&self) -> bool {
        let next = !self.capture_muted.load(Ordering::Relaxed);
        self.capture_muted.store(next, Ordering::Relaxed);
        next
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraySettingsAction {
    ToggleCollection,
    CycleMode,
    SnoozeOneHour,
}

/// Applies one tray mutation while preserving collection consent.
#[must_use]
pub fn apply_tray_settings_action(
    settings: &BehaviorSettingsDto,
    action: TraySettingsAction,
    now_epoch_seconds: i64,
) -> BehaviorSettingsDto {
    let mut next = settings.clone();
    match action {
        TraySettingsAction::ToggleCollection => {
            next.collection_enabled = if settings.collection_enabled {
                false
            } else {
                settings.consent == ConsentStateDto::Accepted
            };
        }
        TraySettingsAction::CycleMode => {
            next.manual_mode_override = cycle_mode(settings.manual_mode_override);
        }
        TraySettingsAction::SnoozeOneHour => {
            next.proactive_snoozed_until = Some(now_epoch_seconds.saturating_add(60 * 60));
        }
    }
    next
}

/// Installs all configured global shortcuts and the system tray.
///
/// Shortcut registration is atomic from the application's perspective: any
/// registration error removes every shortcut registered by this process.
///
/// # Errors
///
/// Returns an error when settings, shortcut parsing/registration, or tray
/// creation fails.
pub fn setup_desktop_controls<R: Runtime>(app: &mut tauri::App<R>) -> Result<(), String> {
    app.manage(DesktopControlState::default());
    app.handle()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .map_err(|error| error.to_string())?;

    let layout = app.state::<pw_platform::paths::AppDataLayout>();
    let settings = load_behavior_settings_checked(&layout).map_err(|error| error.to_string())?;
    if let Err(error) = register_shortcuts(app, &settings.shortcuts) {
        let _ = app.global_shortcut().unregister_all();
        tracing::warn!(%error, "desktop shortcuts are unavailable; all registrations removed");
    }

    build_tray(app).map_err(|error| error.to_string())?;
    Ok(())
}

fn register_shortcuts<R: Runtime>(
    app: &tauri::App<R>,
    settings: &ShortcutSettingsDto,
) -> Result<(), String> {
    ShortcutBindings::from_settings(settings).map_err(str::to_owned)?;
    let shortcut_values = [
        settings.push_to_talk.clone(),
        settings.toggle_mute.clone(),
        settings.open_control_center.clone(),
        settings.toggle_character.clone(),
        settings.cycle_mode.clone(),
    ];
    let shortcuts = shortcut_values
        .iter()
        .map(|value| value.parse::<Shortcut>().map_err(|error| error.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let actions_by_id = shortcuts
        .iter()
        .zip([
            DesktopControlAction::PushToTalk,
            DesktopControlAction::ToggleMute,
            DesktopControlAction::OpenControlCenter,
            DesktopControlAction::ToggleCharacter,
            DesktopControlAction::CycleMode,
        ])
        .map(|(shortcut, action)| (shortcut.id(), action))
        .collect::<HashMap<_, _>>();
    app.global_shortcut()
        .on_shortcuts(shortcuts, move |app, shortcut, event| {
            let Some(action) = actions_by_id.get(&shortcut.id()).copied() else {
                return;
            };
            handle_shortcut(app, action, event.state);
        })
        .map_err(|error| error.to_string())
}

fn handle_shortcut<R: Runtime>(
    app: &AppHandle<R>,
    action: DesktopControlAction,
    state: ShortcutState,
) {
    if action == DesktopControlAction::PushToTalk {
        app.state::<SpeechService>()
            .set_capture_enabled(state == ShortcutState::Pressed);
        return;
    }
    if state != ShortcutState::Pressed {
        return;
    }
    let result = match action {
        DesktopControlAction::PushToTalk => Ok(()),
        DesktopControlAction::ToggleMute => {
            let muted = app.state::<DesktopControlState>().toggle_capture_mute();
            app.state::<SpeechService>().set_capture_enabled(!muted);
            Ok(())
        }
        DesktopControlAction::OpenControlCenter => show_settings(app),
        DesktopControlAction::ToggleCharacter => toggle_window(app, "character"),
        DesktopControlAction::CycleMode => {
            mutate_settings(app, TraySettingsAction::CycleMode).map(|_| ())
        }
    };
    if let Err(error) = result {
        tracing::warn!(%error, ?action, "desktop shortcut action failed");
    }
}

fn build_tray<R: Runtime>(app: &tauri::App<R>) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tauri::tray::TrayIconBuilder;

    let open = MenuItem::with_id(app, "open-settings", "Open settings", true, None::<&str>)?;
    let collection = MenuItem::with_id(
        app,
        "toggle-collection",
        "Pause or resume activity collection",
        true,
        None::<&str>,
    )?;
    let mode = MenuItem::with_id(app, "cycle-mode", "Cycle behavior mode", true, None::<&str>)?;
    let snooze = MenuItem::with_id(
        app,
        "snooze-proactive",
        "Snooze proactive conversation for 1 hour",
        true,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Parallel World", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&open, &collection, &mode, &snooze, &separator, &quit],
    )?;
    let mut builder = TrayIconBuilder::with_id("parallel-world")
        .tooltip("Parallel World")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let result = match event.id().as_ref() {
                "open-settings" => show_settings(app),
                "toggle-collection" => {
                    mutate_settings(app, TraySettingsAction::ToggleCollection).map(|_| ())
                }
                "cycle-mode" => mutate_settings(app, TraySettingsAction::CycleMode).map(|_| ()),
                "snooze-proactive" => {
                    mutate_settings(app, TraySettingsAction::SnoozeOneHour).map(|_| ())
                }
                "quit" => {
                    app.exit(0);
                    Ok(())
                }
                _ => Ok(()),
            };
            if let Err(error) = result {
                tracing::warn!(%error, id = ?event.id(), "tray action failed");
            }
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    Ok(())
}

fn show_settings<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let window = app
        .get_webview_window("settings")
        .ok_or_else(|| "settings window is unavailable".to_owned())?;
    window.show().map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())
}

fn toggle_window<R: Runtime>(app: &AppHandle<R>, label: &str) -> Result<(), String> {
    let window = app
        .get_webview_window(label)
        .ok_or_else(|| format!("{label} window is unavailable"))?;
    let visible = window.is_visible().map_err(|error| error.to_string())?;
    if visible {
        window.hide()
    } else {
        window.show()
    }
    .map_err(|error| error.to_string())
}

fn mutate_settings<R: Runtime>(
    app: &AppHandle<R>,
    action: TraySettingsAction,
) -> Result<BehaviorSettingsDto, String> {
    let layout = app.state::<pw_platform::paths::AppDataLayout>();
    let current = load_behavior_settings_checked(&layout).map_err(|error| error.to_string())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "system time is before the Unix epoch".to_owned())?
        .as_secs();
    let next = apply_tray_settings_action(
        &current,
        action,
        i64::try_from(now).map_err(|_| "system time is out of range".to_owned())?,
    );
    save_behavior_settings(&layout, &next)?;
    app.emit(
        BEHAVIOR_SETTINGS_CHANGED_EVENT,
        BehaviorSettingsChangedEventDto {
            schema_version: BEHAVIOR_SETTINGS_SCHEMA_VERSION,
            settings: next.clone(),
        },
    )
    .map_err(|error| error.to_string())?;
    Ok(next)
}
