//! Pure conversation domain for Parallel World.
//!
//! This crate must stay free of Tauri, HTTP, `SQLite`, OS APIs and
//! sherpa-onnx. It only models conversation concepts and rules.

pub mod conversation;
pub mod reply;
pub mod runtime_health;
pub mod speech;
