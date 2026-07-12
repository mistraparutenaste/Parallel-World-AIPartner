//! Conversation use-case: user utterances in, streamed replies out.

mod orchestrator;
mod ports;
mod prompt;

pub use orchestrator::{ConversationOrchestrator, OrchestratorConfig};
pub use ports::{ChatMessage, ChatRole, ConversationEvents, LlmClient};
pub use prompt::PromptBuilder;
