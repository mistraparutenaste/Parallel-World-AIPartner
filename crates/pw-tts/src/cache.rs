//! WAV file cache for synthesized sentences.
//!
//! Keys are derived from the synthesis inputs (text, style, scales),
//! so a repeated sentence is served from disk without hitting the
//! engine. The cache is pruned oldest-first above a fixed entry count.

use std::io;
use std::path::{Path, PathBuf};

use crate::aivis::SynthesisParams;

/// Default maximum number of cached WAV files.
pub const DEFAULT_MAX_ENTRIES: usize = 200;

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
pub fn cache_key(text: &str, style_id: u32, params: &SynthesisParams) -> String {
    let material = format!(
        "{text}|{style_id}|{volume:.3}|{speed:.3}",
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
        let path = self.path_for(key);
        path.is_file().then_some(path)
    }

    /// Writes WAV bytes for the key and prunes old entries.
    ///
    /// # Errors
    ///
    /// Returns the I/O error when the directory or file cannot be
    /// written. Pruning failures are ignored (best effort).
    pub fn store(&self, key: &str, wav: &[u8]) -> io::Result<PathBuf> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.path_for(key);
        std::fs::write(&path, wav)?;
        self.prune(&path);
        Ok(path)
    }

    /// Removes the oldest files (by modification time) above the
    /// entry limit, never removing `just_written`.
    fn prune(&self, just_written: &Path) {
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{WavCache, cache_key};
    use crate::aivis::SynthesisParams;

    fn params() -> SynthesisParams {
        SynthesisParams::default()
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pw-tts-cache-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn key_is_deterministic_and_input_sensitive() {
        let base = cache_key("こんにちは。", 1, &params());
        assert_eq!(base, cache_key("こんにちは。", 1, &params()));
        assert_ne!(base, cache_key("こんばんは。", 1, &params()));
        assert_ne!(base, cache_key("こんにちは。", 2, &params()));
        assert_ne!(
            base,
            cache_key(
                "こんにちは。",
                1,
                &SynthesisParams {
                    speed: 1.2,
                    ..params()
                }
            )
        );
        assert_eq!(base.len(), 16);
    }

    #[test]
    fn store_then_lookup_round_trips() {
        let cache = WavCache::new(temp_dir("roundtrip"), 10);
        let key = cache_key("やあ", 1, &params());

        assert!(cache.lookup(&key).is_none());
        let path = cache.store(&key, b"RIFFdata").unwrap();
        assert_eq!(cache.lookup(&key), Some(path.clone()));
        assert_eq!(std::fs::read(path).unwrap(), b"RIFFdata");
    }

    #[test]
    fn prunes_oldest_entries_above_the_limit() {
        let cache = WavCache::new(temp_dir("prune"), 3);
        for (index, text) in ["一", "二", "三", "四"].iter().enumerate() {
            let key = cache_key(text, 1, &params());
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

        assert!(cache.lookup(&cache_key("一", 1, &params())).is_none());
        assert!(cache.lookup(&cache_key("二", 1, &params())).is_some());
        assert!(cache.lookup(&cache_key("三", 1, &params())).is_some());
        assert!(cache.lookup(&cache_key("四", 1, &params())).is_some());
    }
}
