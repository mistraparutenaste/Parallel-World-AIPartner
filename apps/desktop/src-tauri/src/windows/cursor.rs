//! Global cursor watcher for the character window.
//!
//! While click-through is enabled the webview receives no mouse
//! events, so the Rust side polls the cursor and streams positions to
//! the character window over IPC; the frontend hit-tests the model
//! and toggles click-through back off when the cursor is over it.

use std::time::Duration;

use pw_contracts::{CharacterCursorEventDto, SCHEMA_VERSION};
use tauri::{AppHandle, Emitter, EventTarget, Manager, PhysicalPosition, PhysicalSize, Runtime};

/// Event carrying [`CharacterCursorEventDto`] payloads.
pub const CURSOR_EVENT: &str = "character-cursor";

const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Converts a global cursor position to CSS coordinates relative to
/// the window client area. Returns `None` when the cursor is outside
/// the window or the scale factor is invalid.
#[must_use]
pub fn relative_css_position(
    cursor: PhysicalPosition<f64>,
    window_position: PhysicalPosition<i32>,
    window_size: PhysicalSize<u32>,
    scale_factor: f64,
) -> Option<(f64, f64)> {
    if scale_factor <= 0.0 {
        return None;
    }
    let x = cursor.x - f64::from(window_position.x);
    let y = cursor.y - f64::from(window_position.y);
    if x < 0.0 || y < 0.0 || x >= f64::from(window_size.width) || y >= f64::from(window_size.height)
    {
        return None;
    }
    Some((x / scale_factor, y / scale_factor))
}

/// Polls the cursor and emits positions to the character window until
/// the window disappears.
pub fn spawn_cursor_watcher<R: Runtime>(app: AppHandle<R>) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(POLL_INTERVAL);
            let Some(window) = app.get_webview_window("character") else {
                break;
            };
            let (Ok(cursor), Ok(position), Ok(size), Ok(scale)) = (
                window.cursor_position(),
                window.inner_position(),
                window.inner_size(),
                window.scale_factor(),
            ) else {
                continue;
            };
            if let Some((x, y)) = relative_css_position(cursor, position, size, scale) {
                let payload = CharacterCursorEventDto {
                    schema_version: SCHEMA_VERSION,
                    x,
                    y,
                };
                if let Err(error) = window.emit_to(
                    EventTarget::webview_window("character"),
                    CURSOR_EVENT,
                    payload,
                ) {
                    tracing::warn!(%error, "failed to emit cursor event");
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use tauri::{PhysicalPosition, PhysicalSize};

    use super::relative_css_position;

    const WINDOW_POSITION: PhysicalPosition<i32> = PhysicalPosition { x: 100, y: 200 };
    const WINDOW_SIZE: PhysicalSize<u32> = PhysicalSize {
        width: 800,
        height: 600,
    };

    #[test]
    fn converts_inside_positions_to_css_coordinates() {
        let cursor = PhysicalPosition { x: 500.0, y: 500.0 };
        let result = relative_css_position(cursor, WINDOW_POSITION, WINDOW_SIZE, 2.0);
        assert_eq!(result, Some((200.0, 150.0)));
    }

    #[test]
    fn rejects_positions_outside_the_window() {
        let left = PhysicalPosition { x: 99.0, y: 300.0 };
        let below = PhysicalPosition { x: 500.0, y: 801.0 };
        assert_eq!(
            relative_css_position(left, WINDOW_POSITION, WINDOW_SIZE, 1.0),
            None
        );
        assert_eq!(
            relative_css_position(below, WINDOW_POSITION, WINDOW_SIZE, 1.0),
            None
        );
    }

    #[test]
    fn rejects_invalid_scale_factors() {
        let cursor = PhysicalPosition { x: 500.0, y: 500.0 };
        assert_eq!(
            relative_css_position(cursor, WINDOW_POSITION, WINDOW_SIZE, 0.0),
            None
        );
    }
}
