//! Runtime directory layout derived from one root directory.

use std::io;
use std::path::PathBuf;

/// All runtime directories of the application, derived from a single
/// root (the platform app data directory).
///
/// Keeping every path here prevents ad-hoc directory creation from
/// being scattered across the codebase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppDataLayout {
    pub root: PathBuf,
    pub config: PathBuf,
    pub data: PathBuf,
    pub models: PathBuf,
    pub characters: PathBuf,
    pub voices: PathBuf,
    pub cache: PathBuf,
    pub logs: PathBuf,
    pub crashes: PathBuf,
    pub tmp: PathBuf,
}

impl AppDataLayout {
    /// Derives the full layout under the given root directory.
    #[must_use]
    pub fn under(root: PathBuf) -> Self {
        Self {
            config: root.join("config"),
            data: root.join("data"),
            models: root.join("models"),
            characters: root.join("characters"),
            voices: root.join("voices"),
            cache: root.join("cache"),
            logs: root.join("logs"),
            crashes: root.join("crashes"),
            tmp: root.join("tmp"),
            root,
        }
    }

    /// Every directory of the layout, in creation order.
    #[must_use]
    pub fn all_directories(&self) -> [&PathBuf; 10] {
        [
            &self.root,
            &self.config,
            &self.data,
            &self.models,
            &self.characters,
            &self.voices,
            &self.cache,
            &self.logs,
            &self.crashes,
            &self.tmp,
        ]
    }

    /// Creates every directory of the layout.
    ///
    /// # Errors
    ///
    /// Returns the first I/O error encountered while creating a
    /// directory.
    pub fn create_all(&self) -> io::Result<()> {
        for directory in self.all_directories() {
            std::fs::create_dir_all(directory)?;
        }
        Ok(())
    }

    /// Conversation history, memory, and companion state database.
    #[must_use]
    pub fn main_database(&self) -> PathBuf {
        self.data.join("parallel-world.sqlite3")
    }

    /// Foreground activity collection database (separate write path).
    #[must_use]
    pub fn activity_database(&self) -> PathBuf {
        self.data.join("activity.sqlite3")
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::AppDataLayout;

    #[test]
    fn derives_all_runtime_directories_from_one_root() {
        let layout = AppDataLayout::under(PathBuf::from("ParallelWorld"));
        assert_eq!(layout.config, PathBuf::from("ParallelWorld/config"));
        assert_eq!(layout.data, PathBuf::from("ParallelWorld/data"));
        assert_eq!(layout.models, PathBuf::from("ParallelWorld/models"));
        assert_eq!(layout.characters, PathBuf::from("ParallelWorld/characters"));
        assert_eq!(layout.voices, PathBuf::from("ParallelWorld/voices"));
        assert_eq!(layout.cache, PathBuf::from("ParallelWorld/cache"));
        assert_eq!(layout.logs, PathBuf::from("ParallelWorld/logs"));
        assert_eq!(layout.crashes, PathBuf::from("ParallelWorld/crashes"));
        assert_eq!(layout.tmp, PathBuf::from("ParallelWorld/tmp"));
    }

    #[test]
    fn create_all_creates_every_directory() {
        let root = std::env::temp_dir().join(format!("pw-layout-test-{}", std::process::id()));
        let layout = AppDataLayout::under(root.clone());
        layout.create_all().expect("create all directories");
        for dir in layout.all_directories() {
            assert!(dir.is_dir(), "missing directory: {}", dir.display());
        }
        std::fs::remove_dir_all(&root).expect("clean up test directories");
    }
}
