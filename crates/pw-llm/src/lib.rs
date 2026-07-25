//! OpenAI-compatible chat completion adapter (llama-server et al.).

mod client;
mod evaluator;

pub use client::{LlmClientConfig, OpenAiCompatClient, SamplingOptions};
pub use evaluator::{
    EVALUATOR_TIMEOUT, EvaluationDecision, EvaluatorConfig, EvaluatorContext, OpenAiCompatEvaluator,
};
// Endpoint policy moved to pw-platform (shared with pw-tts); re-export
// for existing callers.
pub use pw_platform::net::{EndpointError, validate_base_url};
