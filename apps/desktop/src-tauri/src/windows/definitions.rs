//! Static definitions of the three application windows.

use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

/// Declarative description of one application window.
#[allow(clippy::struct_excessive_bools)] // Mirrors Tauri's independent window flags.
pub struct WindowDefinition {
    pub label: &'static str,
    pub title: &'static str,
    pub url: &'static str,
    pub transparent: bool,
    pub decorations: bool,
    pub shadow: bool,
    pub visible: bool,
    pub width: f64,
    pub height: f64,
}

/// The complete window set. Every webview the app ever opens is
/// listed here so capabilities can be pinned per label.
pub const WINDOWS: [WindowDefinition; 3] = [
    WindowDefinition {
        label: "character",
        title: "Parallel World",
        url: "character.html",
        transparent: true,
        decorations: false,
        shadow: false,
        visible: true,
        width: 480.0,
        height: 720.0,
    },
    WindowDefinition {
        label: "chat",
        title: "Parallel World - チャット",
        url: "chat.html",
        transparent: false,
        decorations: true,
        shadow: true,
        visible: false,
        width: 420.0,
        height: 640.0,
    },
    WindowDefinition {
        label: "settings",
        title: "Parallel World - 設定",
        url: "settings.html",
        transparent: false,
        decorations: true,
        shadow: true,
        visible: true,
        width: 720.0,
        height: 560.0,
    },
];

/// Creates every window from [`WINDOWS`] that does not exist yet.
///
/// # Errors
///
/// Returns an error when a webview window cannot be created.
pub fn create_missing_windows<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    for definition in &WINDOWS {
        if app.get_webview_window(definition.label).is_none() {
            WebviewWindowBuilder::new(
                app,
                definition.label,
                WebviewUrl::App(definition.url.into()),
            )
            .title(definition.title)
            .transparent(definition.transparent)
            .decorations(definition.decorations)
            .shadow(definition.shadow)
            .visible(definition.visible)
            .inner_size(definition.width, definition.height)
            .build()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn defines_exactly_three_unique_window_labels() {
        let labels: Vec<_> = super::WINDOWS.iter().map(|window| window.label).collect();
        assert_eq!(labels, ["character", "chat", "settings"]);
    }

    #[test]
    fn only_the_character_window_is_transparent_undecorated_and_shadowless() {
        for window in &super::WINDOWS {
            let is_character = window.label == "character";
            assert_eq!(window.transparent, is_character, "{}", window.label);
            assert_eq!(window.decorations, !is_character, "{}", window.label);
            assert_eq!(window.shadow, !is_character, "{}", window.label);
            assert_eq!(window.visible, window.label != "chat", "{}", window.label);
        }
    }
}
