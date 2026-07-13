//! Mandatory signed update checks and installation orchestration.

pub mod backend;
pub mod service;

pub use backend::{SettingsUpdateEmitter, TauriUpdateBackend};
pub use service::{IdempotentFlusher, UpdateService};
