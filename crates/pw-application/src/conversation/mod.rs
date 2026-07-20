//! Conversation use-case: user utterances in, streamed replies out.

mod orchestrator;
mod ports;
mod prompt;
mod routing;

pub use orchestrator::{ConversationOrchestrator, OrchestratorConfig};
pub use ports::{ChatMessage, ChatRole, ConversationEvents, LlmClient};
pub use prompt::PromptBuilder;
pub use routing::{
    ConfiguredResponsePipeline, ExistingContextRetriever, FixedSurfaceRealizer, IntentRouter,
    LexicalResponsePlanner, PlanningBudget, PreparedResponse, ResponseContextRetriever,
    ResponsePipeline, ResponsePlan, ResponsePlanner, SurfaceContext, SurfaceRealizer, TurnKind,
    default_response_pipeline, response_pipeline,
};
