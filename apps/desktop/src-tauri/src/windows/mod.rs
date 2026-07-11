//! Window definitions, creation and cursor watching.

mod cursor;
mod definitions;

pub use cursor::{CURSOR_EVENT, relative_css_position, spawn_cursor_watcher};
pub use definitions::{WINDOWS, WindowDefinition, create_missing_windows};
