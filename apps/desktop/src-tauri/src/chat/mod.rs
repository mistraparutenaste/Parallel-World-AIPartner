//! Conversation service wiring (LLM adapter, prompts, events).

mod service;
mod settings;

pub use service::ChatService;
pub use settings::{default_llm_settings, load_llm_api_key, load_llm_settings, save_llm_settings};
