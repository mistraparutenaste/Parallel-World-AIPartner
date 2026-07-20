//! Window definitions, creation and cursor watching.

mod cursor;
mod definitions;
mod lifecycle;

pub use cursor::{CURSOR_EVENT, relative_css_position, spawn_cursor_watcher};
pub use definitions::{
    CHARACTER_WINDOW_HEIGHT, CHARACTER_WINDOW_WIDTH, WINDOWS, WindowDefinition,
    apply_character_window_size, create_missing_windows,
};
pub use lifecycle::should_exit_after_close;
