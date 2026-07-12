#![forbid(unsafe_code)]

mod database;
mod history;

pub use database::{Database, StorageError};
pub use history::SqliteConversationHistory;
