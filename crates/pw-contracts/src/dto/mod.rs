mod app_status;
mod character_presentation;

pub use app_status::{AppStatusDto, ConversationStateDto, SCHEMA_VERSION};
pub use character_presentation::{
    CHARACTER_PRESENTATION_SCHEMA_VERSION, CharacterPresentationSettingsDto,
};
