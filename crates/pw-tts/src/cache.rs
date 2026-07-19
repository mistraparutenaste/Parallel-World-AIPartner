//! WAV file cache for synthesized sentences.
//!
//! Keys are derived from the synthesis inputs (engine, voice, text, scales),
//! so a repeated sentence is served from disk without hitting the
//! engine. The cache is pruned oldest-first above a fixed entry count.

use std::io;
use std::path::{Path, PathBuf};

use crate::aivis::SynthesisParams;

/// Default maximum number of cached WAV files.
pub const DEFAULT_MAX_ENTRIES: usize = 200;

/// Aggregate size of application-owned synthesized WAV files.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WavCacheStats {
    pub files: u64,
    pub bytes: u64,
}

/// Failure while removing synthesized WAV files.
#[derive(Debug, thiserror::Error)]
pub enum WavCacheClearError {
    #[error("failed to enumerate tts cache: {source}")]
    Enumerate {
        #[source]
        source: io::Error,
    },
    #[error(
        "tts cache clear partially failed: deleted {deleted_files} files ({deleted_bytes} bytes); failed {failed_files} files; first error: {source}"
    )]
    Partial {
        deleted_files: u64,
        deleted_bytes: u64,
        failed_files: u64,
        #[source]
        source: io::Error,
    },
}

/// Disk cache of synthesized WAV files under one directory.
#[derive(Debug, Clone)]
pub struct WavCache {
    dir: PathBuf,
    max_entries: usize,
}

/// FNV-1a 64-bit hash (deterministic across runs, unlike `DefaultHasher`).
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Cache key for one synthesis request.
#[must_use]
pub fn cache_key(engine: &str, voice_id: &str, text: &str, params: &SynthesisParams) -> String {
    let material = format!(
        "{engine}|{voice_id}|{text}|{volume:.3}|{speed:.3}",
        volume = params.volume,
        speed = params.speed
    );
    format!("{:016x}", fnv1a64(material.as_bytes()))
}

impl WavCache {
    /// Creates the cache rooted at `dir` (created lazily on store).
    #[must_use]
    pub fn new(dir: PathBuf, max_entries: usize) -> Self {
        Self { dir, max_entries }
    }

    /// Path a given key would be stored at.
    #[must_use]
    pub fn path_for(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.wav"))
    }

    /// Returns the cached file for the key, if present.
    #[must_use]
    pub fn lookup(&self, key: &str) -> Option<PathBuf> {
        if !self.validate_root_if_present().unwrap_or(false) {
            return None;
        }
        let path = self.path_for(key);
        path.is_file().then_some(path)
    }

    /// Counts regular `.wav` files owned by this cache.
    ///
    /// A missing cache directory is treated as an empty cache.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when an existing cache directory cannot be read.
    pub fn stats(&self) -> io::Result<WavCacheStats> {
        self.wav_files().map(|files| WavCacheStats {
            files: files.len() as u64,
            bytes: files
                .iter()
                .fold(0_u64, |total, (_, bytes)| total.saturating_add(*bytes)),
        })
    }

    /// Removes every regular `.wav` file owned by this cache.
    ///
    /// Non-WAV files and nested directories are left untouched.
    ///
    /// # Errors
    ///
    /// Returns an enumeration error before deletion starts, or a partial-progress
    /// error after attempting every discovered WAV file.
    pub fn clear(&self) -> Result<WavCacheStats, WavCacheClearError> {
        self.clear_with(|path| std::fs::remove_file(path))
    }

    fn clear_with(
        &self,
        mut remove: impl FnMut(&Path) -> io::Result<()>,
    ) -> Result<WavCacheStats, WavCacheClearError> {
        let files = self
            .wav_files()
            .map_err(|source| WavCacheClearError::Enumerate { source })?;
        let mut deleted = WavCacheStats::default();
        let mut failed_files = 0_u64;
        let mut first_error = None;
        for (path, bytes) in files {
            match remove(&path) {
                Ok(()) => {
                    deleted.files = deleted.files.saturating_add(1);
                    deleted.bytes = deleted.bytes.saturating_add(bytes);
                }
                Err(error) => {
                    failed_files = failed_files.saturating_add(1);
                    first_error.get_or_insert(error);
                }
            }
        }
        if let Some(source) = first_error {
            return Err(WavCacheClearError::Partial {
                deleted_files: deleted.files,
                deleted_bytes: deleted.bytes,
                failed_files,
                source,
            });
        }
        Ok(deleted)
    }

    fn wav_files(&self) -> io::Result<Vec<(PathBuf, u64)>> {
        if !self.validate_root_if_present()? {
            return Ok(Vec::new());
        }
        let entries = std::fs::read_dir(&self.dir)?;
        let mut files = Vec::new();
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().is_some_and(|extension| extension == "wav") {
                files.push((path, entry.metadata()?.len()));
            }
        }
        files.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(files)
    }

    fn validate_root_if_present(&self) -> io::Result<bool> {
        let metadata = match std::fs::symlink_metadata(&self.dir) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        self.validate_existing_root(&metadata)?;
        Ok(true)
    }

    fn ensure_valid_root(&self) -> io::Result<()> {
        if self.validate_root_if_present()? {
            return Ok(());
        }
        std::fs::create_dir_all(&self.dir)?;
        let metadata = std::fs::symlink_metadata(&self.dir)?;
        self.validate_existing_root(&metadata)
    }

    fn validate_existing_root(&self, metadata: &std::fs::Metadata) -> io::Result<()> {
        if metadata.file_type().is_symlink() || is_windows_reparse_point(metadata) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "refusing to follow tts cache symlink or reparse point: {}",
                    self.dir.display()
                ),
            ));
        }
        if !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("tts cache root is not a directory: {}", self.dir.display()),
            ));
        }
        let parent = self
            .dir
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let canonical_parent = parent.canonicalize()?;
        let canonical_root = self.dir.canonicalize()?;
        if canonical_root.parent() != Some(canonical_parent.as_path()) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "tts cache root resolves outside its expected parent: {}",
                    self.dir.display()
                ),
            ));
        }
        Ok(())
    }

    /// Writes WAV bytes for the key and prunes old entries.
    ///
    /// # Errors
    ///
    /// Returns the I/O error when the directory or file cannot be
    /// written. Pruning failures are ignored (best effort).
    pub fn store(&self, key: &str, wav: &[u8]) -> io::Result<PathBuf> {
        self.ensure_valid_root()?;
        let path = self.path_for(key);
        std::fs::write(&path, wav)?;
        self.prune(&path);
        Ok(path)
    }

    /// Removes the oldest files (by modification time) above the
    /// entry limit, never removing `just_written`.
    fn prune(&self, just_written: &Path) {
        match self.validate_root_if_present() {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => {
                tracing::debug!(%error, path = ?self.dir, "refusing to prune invalid tts cache root");
                return;
            }
        }
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "wav") && path != just_written)
            .filter_map(|path| {
                let modified = path.metadata().and_then(|meta| meta.modified()).ok()?;
                Some((modified, path))
            })
            .collect();
        // just_written always survives, so the others may keep at most
        // max_entries - 1 slots.
        let keep = self.max_entries.saturating_sub(1);
        if files.len() <= keep {
            return;
        }
        files.sort_by_key(|(modified, _)| *modified);
        let excess = files.len() - keep;
        for (_, path) in files.into_iter().take(excess) {
            if let Err(error) = std::fs::remove_file(&path) {
                tracing::debug!(%error, ?path, "failed to prune tts cache entry");
            }
        }
    }
}

#[cfg(windows)]
fn is_windows_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_windows_reparse_point(_: &std::fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::PathBuf;

    use super::{WavCache, WavCacheClearError, WavCacheStats, cache_key};
    use crate::aivis::SynthesisParams;

    fn params() -> SynthesisParams {
        SynthesisParams::default()
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pw-tts-cache-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[cfg(windows)]
    fn create_directory_link(target: &std::path::Path, link: &std::path::Path) -> bool {
        use std::os::windows::fs::symlink_dir;

        if symlink_dir(target, link).is_ok() {
            return true;
        }
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .is_ok_and(|output| output.status.success())
    }

    #[test]
    fn key_separates_engines_and_voice_ids() {
        let aivis = cache_key("aivis", "1", "hello", &params());

        assert_eq!(aivis, cache_key("aivis", "1", "hello", &params()));
        assert_ne!(aivis, cache_key("irodori", "1", "hello", &params()));
        assert_ne!(aivis, cache_key("aivis", "2", "hello", &params()));
        assert_ne!(aivis, cache_key("aivis", "1", "goodbye", &params()));
    }

    #[test]
    fn key_is_deterministic_and_input_sensitive() {
        let base = cache_key("aivis", "1", "こんにちは。", &params());
        assert_eq!(base, cache_key("aivis", "1", "こんにちは。", &params()));
        assert_ne!(base, cache_key("aivis", "1", "こんばんは。", &params()));
        assert_ne!(base, cache_key("aivis", "2", "こんにちは。", &params()));
        assert_ne!(
            base,
            cache_key(
                "aivis",
                "1",
                "こんにちは。",
                &SynthesisParams {
                    speed: 1.2,
                    ..params()
                }
            )
        );
        assert_ne!(
            base,
            cache_key(
                "aivis",
                "1",
                "こんにちは。",
                &SynthesisParams {
                    volume: 0.8,
                    ..params()
                }
            )
        );
        assert_eq!(base.len(), 16);
    }

    #[test]
    fn store_then_lookup_round_trips() {
        let cache = WavCache::new(temp_dir("roundtrip"), 10);
        let key = cache_key("aivis", "1", "やあ", &params());

        assert!(cache.lookup(&key).is_none());
        let path = cache.store(&key, b"RIFFdata").unwrap();
        assert_eq!(cache.lookup(&key), Some(path.clone()));
        assert_eq!(std::fs::read(path).unwrap(), b"RIFFdata");
    }

    #[test]
    fn prunes_oldest_entries_above_the_limit() {
        let cache = WavCache::new(temp_dir("prune"), 3);
        for (index, text) in ["一", "二", "三", "四"].iter().enumerate() {
            let key = cache_key("aivis", "1", text, &params());
            cache.store(&key, b"RIFF").unwrap();
            // Distinct mtimes so the prune order is stable.
            let mtime = std::time::SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(1 + index as u64);
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(cache.path_for(&key))
                .unwrap();
            file.set_modified(mtime).unwrap();
        }

        assert!(
            cache
                .lookup(&cache_key("aivis", "1", "一", &params()))
                .is_none()
        );
        assert!(
            cache
                .lookup(&cache_key("aivis", "1", "二", &params()))
                .is_some()
        );
        assert!(
            cache
                .lookup(&cache_key("aivis", "1", "三", &params()))
                .is_some()
        );
        assert!(
            cache
                .lookup(&cache_key("aivis", "1", "四", &params()))
                .is_some()
        );
    }

    #[test]
    fn stats_and_clear_only_include_owned_wav_files() {
        let dir = temp_dir("clear");
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join("one.wav"), b"1234").unwrap();
        std::fs::write(dir.join("two.wav"), b"123456").unwrap();
        std::fs::write(dir.join("keep.txt"), b"keep").unwrap();
        std::fs::write(dir.join("nested/keep.wav"), b"nested").unwrap();
        let cache = WavCache::new(dir.clone(), 10);

        assert_eq!(
            cache.stats().unwrap(),
            WavCacheStats {
                files: 2,
                bytes: 10
            }
        );
        assert_eq!(
            cache.clear().unwrap(),
            WavCacheStats {
                files: 2,
                bytes: 10
            }
        );
        assert_eq!(cache.stats().unwrap(), WavCacheStats::default());
        assert!(dir.join("keep.txt").exists());
        assert!(dir.join("nested/keep.wav").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn clear_attempts_every_file_and_reports_partial_progress() {
        let dir = temp_dir("partial-clear");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("one.wav"), b"1234").unwrap();
        std::fs::write(dir.join("two.wav"), b"123456").unwrap();
        std::fs::write(dir.join("three.wav"), b"12345678").unwrap();
        let cache = WavCache::new(dir.clone(), 10);
        let mut attempted = Vec::new();

        let error = cache
            .clear_with(|path| {
                attempted.push(path.file_name().unwrap().to_owned());
                if path.file_name().is_some_and(|name| name == "two.wav") {
                    Err(io::Error::new(io::ErrorKind::PermissionDenied, "locked"))
                } else {
                    std::fs::remove_file(path)
                }
            })
            .unwrap_err();

        assert_eq!(attempted.len(), 3);
        assert!(dir.join("two.wav").exists());
        assert!(!dir.join("one.wav").exists());
        assert!(!dir.join("three.wav").exists());
        assert!(matches!(
            &error,
            WavCacheClearError::Partial {
                deleted_files: 2,
                deleted_bytes: 12,
                failed_files: 1,
                ..
            }
        ));
        assert!(error.to_string().contains("deleted 2 files (12 bytes)"));
        assert!(error.to_string().contains("failed 1 files"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn clear_rejects_a_symlink_cache_root_without_touching_the_target() {
        use std::os::unix::fs::symlink;

        let root = temp_dir("symlink-root");
        let target = root.join("outside");
        let link = root.join("tts");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("keep.wav"), b"outside").unwrap();
        symlink(&target, &link).unwrap();

        let error = WavCache::new(link, 10).clear().unwrap_err();
        assert!(matches!(error, WavCacheClearError::Enumerate { .. }));
        assert!(target.join("keep.wav").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn cache_io_rejects_a_symlink_root_without_touching_the_target() {
        use std::os::unix::fs::symlink;

        let root = temp_dir("store-symlink-root");
        let target = root.join("outside");
        let link = root.join("tts");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("keep.wav"), b"outside").unwrap();
        symlink(&target, &link).unwrap();

        let cache = WavCache::new(link, 1);
        assert!(cache.lookup("keep").is_none());
        cache.prune(&cache.path_for("new"));
        assert_eq!(std::fs::read(target.join("keep.wav")).unwrap(), b"outside");
        let result = cache.store("new", b"RIFF");
        assert!(matches!(
            result,
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied
        ));
        assert_eq!(std::fs::read(target.join("keep.wav")).unwrap(), b"outside");
        assert!(!target.join("new.wav").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[test]
    fn clear_rejects_a_directory_symlink_cache_root_when_supported() {
        let root = temp_dir("symlink-root");
        let target = root.join("outside");
        let link = root.join("tts");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("keep.wav"), b"outside").unwrap();
        if !create_directory_link(&target, &link) {
            let _ = std::fs::remove_dir_all(root);
            return;
        }

        let error = WavCache::new(link, 10).clear().unwrap_err();
        assert!(matches!(error, WavCacheClearError::Enumerate { .. }));
        assert!(target.join("keep.wav").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[test]
    fn cache_io_rejects_a_directory_symlink_root_without_touching_the_target() {
        let root = temp_dir("store-symlink-root");
        let target = root.join("outside");
        let link = root.join("tts");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("keep.wav"), b"outside").unwrap();
        if !create_directory_link(&target, &link) {
            let _ = std::fs::remove_dir_all(root);
            return;
        }

        let cache = WavCache::new(link, 1);
        assert!(cache.lookup("keep").is_none());
        cache.prune(&cache.path_for("new"));
        assert_eq!(std::fs::read(target.join("keep.wav")).unwrap(), b"outside");
        let result = cache.store("new", b"RIFF");
        let kept = std::fs::read(target.join("keep.wav"));
        let wrote_new = target.join("new.wav").exists();
        let _ = std::fs::remove_dir_all(root);

        assert!(matches!(
            result,
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied
        ));
        assert_eq!(kept.unwrap(), b"outside");
        assert!(!wrote_new);
    }
}
