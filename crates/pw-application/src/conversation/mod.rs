//! Conversation use-case: user utterances in, streamed replies out.

mod orchestrator;
mod ports;
mod prompt;
mod routing;

pub use orchestrator::{ConversationOrchestrator, OrchestratorConfig};
pub use ports::{ChatMessage, ChatRole, ConversationEvents, LlmClient};
pub use prompt::PromptBuilder;
pub use routing::{
    BoundedStateContext, ClosingPreference, ConfiguredResponsePipeline, DialogueClassifier,
    DialogueTurnKind, ExistingContextRetriever, FixedSurfaceRealizer, IntentRouter,
    LexicalResponsePlanner, PlannedStateContextProvider, PlanningBudget, PreparedResponse,
    QuestionPolicy, ResponseContextRetriever, ResponsePipeline, ResponsePlan, ResponsePlanner,
    StateAwareRetriever, SurfaceContext, SurfaceRealizer, TurnKind, TurnStyleContract,
    default_response_pipeline, response_pipeline,
};
