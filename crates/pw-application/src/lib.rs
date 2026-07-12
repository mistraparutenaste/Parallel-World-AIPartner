//! Application services: use-case orchestration over the domain,
//! with all external capabilities abstracted behind ports.

pub mod conversation;
mod port_error;
pub mod speech;
pub mod speech_synthesis;

pub use port_error::PortError;
