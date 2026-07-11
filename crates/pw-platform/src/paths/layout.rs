use std::{fs, io, path::PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppDataLayout {
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
        }
    }

    #[must_use]
    pub fn directories(&self) -> [&PathBuf; 9] {
        [
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

    pub fn create_all(&self) -> io::Result<()> {
        for directory in self.directories() {
            fs::create_dir_all(directory)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

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
    fn creates_every_runtime_directory() {
        let root = std::env::temp_dir().join(format!(
            "parallel-world-layout-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should follow Unix epoch")
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        let layout = AppDataLayout::under(root.clone());

        layout
            .create_all()
            .expect("runtime directories should be created");

        for directory in layout.directories() {
            assert!(
                directory.is_dir(),
                "{} was not created",
                directory.display()
            );
        }

        fs::remove_dir_all(root).expect("test directory should be removable");
    }
}
