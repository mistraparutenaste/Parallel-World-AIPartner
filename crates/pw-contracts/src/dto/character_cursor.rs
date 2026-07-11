//! Cursor position event contract for the character window.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Cursor position relative to the character window client area, in
/// CSS pixels. Streamed by the Rust cursor watcher so the window can
/// hit-test even while cursor events are ignored (click-through).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "CharacterCursorEventDto.ts")]
pub struct CharacterCursorEventDto {
    pub schema_version: u16,
    pub x: f64,
    pub y: f64,
}

#[cfg(test)]
mod tests {
    use super::CharacterCursorEventDto;
    use crate::SCHEMA_VERSION;

    #[test]
    fn serializes_cursor_event_contract() {
        let value = CharacterCursorEventDto {
            schema_version: SCHEMA_VERSION,
            x: 12.5,
            y: 34.0,
        };
        let json = serde_json::to_value(value).unwrap();
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["x"], 12.5);
        assert_eq!(json["y"], 34.0);
    }
}
