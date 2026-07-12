//! OpenAI-compatible chat completion adapter (llama-server et al.).

mod client;

pub use client::{LlmClientConfig, OpenAiCompatClient};
// Endpoint policy moved to pw-platform (shared with pw-tts); re-export
// for existing callers.
pub use pw_platform::net::{EndpointError, validate_base_url};
