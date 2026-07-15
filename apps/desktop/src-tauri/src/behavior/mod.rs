//! Context-aware companion settings persistence.

mod activity;
mod atomic_json;
mod mode;
mod personas;
mod settings;

pub use activity::{
    ActivityClock, ActivityCollector, ActivityCollectorError, ActivityCollectorService,
    ActivityCollectorStartError, ActivityRepository, ActivityRepositoryError,
    ActivitySettingsSource, ActivitySettingsSourceError,
};
pub use mode::{ModeResolutionError, ModeResolutionInput, ResolvedMode, resolve_mode};
#[cfg(test)]
pub(crate) use personas::PersonaPromptSource;
pub(crate) use personas::{ResolvedPersonaPrompt, resolve_persona_prompt};
pub use personas::{load_persona, migrate_legacy_character_prompt, save_persona_settings};
pub use settings::{
    BehaviorSettingsLoadError, load_behavior_settings, load_behavior_settings_checked,
    save_behavior_settings,
};
