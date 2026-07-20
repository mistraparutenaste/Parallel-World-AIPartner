#![forbid(unsafe_code)]

pub mod activity;
mod database;
mod history;
mod memory;
mod state;
mod state_worker;

pub use database::{Database, StorageError};
pub use history::SqliteConversationHistory;
pub use memory::{
    SqliteMemoryStore, delete_all_memories_fenced, delete_all_memories_in_transaction,
    tombstone_memories_for_deleted_observations,
};
pub use state::SqliteCompanionStateStore;
pub use state_worker::{
    CompanionStateWorker, DEFAULT_STATE_QUEUE_CAPACITY, SqlitePlannedStateContext,
    apply_async_state_write,
};
