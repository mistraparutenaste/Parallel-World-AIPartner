//! Context-aware companion settings persistence.

mod activity;
mod mode;
mod personas;
mod proactive_runtime;
mod runtime;
mod safety;
mod settings;

pub use activity::{
    ActivityClock, ActivityCollector, ActivityCollectorError, ActivityCollectorService,
    ActivityCollectorStartError, ActivityRepository, ActivityRepositoryError,
    ActivitySettingsSource, ActivitySettingsSourceError,
};
pub use mode::{ModeResolutionError, ModeResolutionInput, ResolvedMode, resolve_mode};
#[cfg(test)]
pub(crate) use personas::PersonaPromptSource;
pub(crate) use personas::{ResolvedPersonaPrompt, resolve_persona_prompt_with_pause};
pub use personas::{
    load_persona, load_persona_checked, migrate_legacy_character_prompt, save_persona,
    save_persona_settings,
};
pub use proactive_runtime::{
    BehaviorRuntimeService, ProactiveDeliveryDecision, ProactiveDeliveryInput,
    decide_proactive_delivery,
};
pub use runtime::{
    BehaviorRuntimeSnapshot, RuntimeCollectionHealth, RuntimeMode, resolve_runtime_snapshot,
};
pub use safety::{
    DarkExpressionSafetyLoadError, DarkExpressionSafetyState, load_dark_expression_safety,
    load_dark_expression_safety_checked, safe_word_matches, sanitize_safe_word,
    save_dark_expression_safety,
};
pub use settings::{
    BehaviorSettingsLoadError, load_behavior_settings, load_behavior_settings_checked,
    save_behavior_settings,
};
