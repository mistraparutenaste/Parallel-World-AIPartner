//! IPC data transfer objects.

mod app_status;
mod character_cursor;
mod character_manifest;

pub use app_status::{AppStatusDto, ConversationStateDto, SCHEMA_VERSION};
pub use character_cursor::CharacterCursorEventDto;
pub use character_manifest::{CharacterManifestDto, MotionGroupDto};
