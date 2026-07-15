//! Application services: use-case orchestration over the domain,
//! with all external capabilities abstracted behind ports.

pub mod behavior;
pub mod conversation;
pub mod history;
pub mod memory;
mod port_error;
pub mod recovery;
pub mod speech;
pub mod speech_synthesis;
pub mod stability;

pub use port_error::PortError;
