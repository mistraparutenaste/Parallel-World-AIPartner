//! OpenAI-compatible chat completion adapter (llama-server et al.).

mod client;
mod endpoint;

pub use client::{LlmClientConfig, OpenAiCompatClient};
pub use endpoint::EndpointError;
