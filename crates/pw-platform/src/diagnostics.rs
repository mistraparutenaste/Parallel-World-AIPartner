//! Redacted, bounded crash diagnostics persisted with atomic replacement.

use serde::{Deserialize, Serialize};
#[cfg(windows)]
mod windows_atomic;
use std::{
    backtrace::Backtrace,
    cell::Cell,
    fs, io,
    io::Write,
    panic::PanicHookInfo,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

pub const DEFAULT_MAX_FILES: usize = 20;
pub const DEFAULT_MAX_BYTES: u64 = 20 * 1024 * 1024;
static REPORT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static PANIC_COUNT: AtomicU64 = AtomicU64::new(0);
static STORE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug)]
pub struct RetentionPolicy {
    pub max_files: usize,
    pub max_bytes: u64,
}
impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            max_files: DEFAULT_MAX_FILES,
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CrashInput {
    source: &'static str,
    category: String,
    detail: String,
    thread: Option<String>,
    location: Option<String>,
    backtrace: Option<String>,
}

impl CrashInput {
    #[must_use]
    pub fn frontend(category: &'static str, line: Option<u32>, column: Option<u32>) -> Self {
        Self {
            source: "frontend",
            category: category.into(),
            detail: format!(
                "line={};column={}",
                line.unwrap_or_default(),
                column.unwrap_or_default()
            ),
            thread: None,
            location: None,
            backtrace: None,
        }
    }

    fn panic(info: &PanicHookInfo<'_>) -> Self {
        let category = if info.payload().downcast_ref::<&str>().is_some() {
            "panic_str"
        } else if info.payload().downcast_ref::<String>().is_some() {
            "panic_string"
        } else {
            "panic_non_string"
        };
        Self {
            source: "rust",
            category: category.into(),
            detail: "panic payload omitted".into(),
            thread: std::thread::current().name().map(redact),
            location: info
                .location()
                .map(|v| format!("{}:{}:{}", v.file(), v.line(), v.column()))
                .map(|v| redact(&v)),
            backtrace: capture_backtrace(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiagnosticEntry {
    pub schema_version: u16,
    pub id: String,
    pub timestamp_ms: u64,
    pub category: String,
    pub bytes: u64,
}

#[derive(Serialize)]
struct CrashReport {
    schema_version: u32,
    timestamp_ms: u64,
    build: &'static str,
    source: &'static str,
    payload_category: String,
    detail: String,
    thread: Option<String>,
    location: Option<String>,
    backtrace: Option<String>,
}

#[derive(Clone, Debug)]
pub struct DiagnosticStore {
    directory: PathBuf,
    policy: RetentionPolicy,
}

impl DiagnosticStore {
    #[must_use]
    pub fn new(directory: impl Into<PathBuf>, policy: RetentionPolicy) -> Self {
        Self {
            directory: directory.into(),
            policy,
        }
    }

    /// # Errors
    /// Returns an I/O or serialization error if the report cannot be persisted.
    pub fn write(&self, input: CrashInput) -> Result<PathBuf, DiagnosticError> {
        let _guard = STORE_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let path = self.write_unpruned(input)?;
        self.prune()?;
        Ok(path)
    }

    fn write_unpruned(&self, input: CrashInput) -> Result<PathBuf, DiagnosticError> {
        fs::create_dir_all(&self.directory)?;
        let timestamp_ms = now_ms();
        let id = format!(
            "crash-{timestamp_ms}-{}-{}.json",
            std::process::id(),
            REPORT_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let destination = self.directory.join(&id);
        let report = CrashReport {
            schema_version: 1,
            timestamp_ms,
            build: env!("CARGO_PKG_VERSION"),
            source: input.source,
            payload_category: input.category,
            detail: input.detail,
            thread: input.thread,
            location: input.location,
            backtrace: input.backtrace,
        };
        let bytes = serde_json::to_vec_pretty(&report)?;
        let (temporary_path, mut temporary) = unique_temp(&self.directory, ".pw-crash-")?;
        temporary.write_all(&bytes)?;
        temporary.sync_all()?;
        drop(temporary);
        if let Err(error) = fs::rename(&temporary_path, &destination) {
            let _ = fs::remove_file(&temporary_path);
            return Err(error.into());
        }
        Ok(destination)
    }

    /// # Errors
    /// Returns an I/O error if the report directory cannot be read.
    pub fn list(&self) -> Result<Vec<DiagnosticEntry>, DiagnosticError> {
        if !self.directory.exists() {
            return Ok(Vec::new());
        }
        let mut entries = Vec::new();
        for item in fs::read_dir(&self.directory)? {
            let item = item?;
            let path = item.path();
            if path.extension().and_then(|v| v.to_str()) != Some("json")
                || !item.file_type()?.is_file()
            {
                continue;
            }
            let id = item.file_name().to_string_lossy().into_owned();
            let value: serde_json::Value =
                serde_json::from_slice(&fs::read(&path)?).unwrap_or_default();
            entries.push(DiagnosticEntry {
                schema_version: 1,
                id,
                timestamp_ms: value["timestamp_ms"].as_u64().unwrap_or_default(),
                category: value["payload_category"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_owned(),
                bytes: item.metadata()?.len(),
            });
        }
        entries.sort_by(|left, right| {
            right
                .timestamp_ms
                .cmp(&left.timestamp_ms)
                .then_with(|| right.id.cmp(&left.id))
        });
        Ok(entries)
    }

    fn prune(&self) -> Result<(), DiagnosticError> {
        let mut entries = self.list()?;
        let mut total: u64 = entries.iter().map(|v| v.bytes).sum();
        while entries.len() > self.policy.max_files || total > self.policy.max_bytes {
            if let Some(oldest) = entries.pop() {
                total = total.saturating_sub(oldest.bytes);
                fs::remove_file(self.directory.join(oldest.id))?;
            } else {
                break;
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Applies the configured retention limits outside latency-sensitive hooks.
    /// # Errors
    /// Returns an I/O error if old reports cannot be removed.
    pub fn maintain(&self) -> Result<(), DiagnosticError> {
        let _guard = STORE_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.prune()
    }

    /// Removes incomplete files left by a terminated process, then applies retention.
    ///
    /// Call this during single-threaded startup, before this store is exposed to panic
    /// hooks or frontend commands. Removing temporary files during normal maintenance
    /// could race an in-progress panic report.
    /// # Errors
    /// Returns an I/O error if the directory cannot be inspected or cleaned.
    pub fn recover_after_unclean_shutdown(&self) -> Result<(), DiagnosticError> {
        let _guard = STORE_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.directory.exists() {
            for item in fs::read_dir(&self.directory)? {
                let item = item?;
                let name = item.file_name();
                let name = name.to_string_lossy();
                if item.file_type()?.is_file()
                    && name.starts_with(".pw-crash-")
                    && name.ends_with(".tmp")
                {
                    fs::remove_file(item.path())?;
                }
            }
        }
        self.prune()
    }

    /// Exports the already-redacted reports as one JSON array using an atomic rename.
    /// # Errors
    /// Returns an error for unsafe destinations or failed I/O.
    pub fn export(&self, destination: &Path, allow_overwrite: bool) -> Result<(), DiagnosticError> {
        let crash_dir = self.directory.canonicalize()?;
        let parent = destination
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .canonicalize()?;
        if parent == crash_dir || parent.starts_with(&crash_dir) {
            return Err(DiagnosticError::UnsafeDestination);
        }
        if destination.exists() {
            if !destination.symlink_metadata()?.file_type().is_file() {
                return Err(DiagnosticError::UnsafeDestination);
            }
            if !allow_overwrite {
                return Err(DiagnosticError::DestinationExists);
            }
        }
        let mut reports = Vec::new();
        for entry in self.list()? {
            reports.push(serde_json::from_slice::<serde_json::Value>(&fs::read(
                self.directory.join(entry.id),
            )?)?);
        }
        let (temporary_path, mut temporary) = unique_temp(&parent, ".pw-export-")?;
        temporary.write_all(&serde_json::to_vec_pretty(&reports)?)?;
        temporary.sync_all()?;
        drop(temporary);
        let result = if allow_overwrite {
            atomic_replace(&temporary_path, destination)
        } else {
            fs::hard_link(&temporary_path, destination)
                .and_then(|()| fs::remove_file(&temporary_path))
        };
        if let Err(error) = result {
            let _ = fs::remove_file(&temporary_path);
            return Err(error.into());
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DiagnosticError {
    #[error("diagnostic I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("diagnostic serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("DESTINATION_EXISTS")]
    DestinationExists,
    #[error("unsafe diagnostic export destination")]
    UnsafeDestination,
}

static PANIC_STORE: OnceLock<RwLock<Arc<DiagnosticStore>>> = OnceLock::new();
static MAINTENANCE_TX: OnceLock<std::sync::mpsc::SyncSender<()>> = OnceLock::new();
thread_local! { static IN_PANIC_HOOK: Cell<bool> = const { Cell::new(false) }; }

/// Installs the process panic hook at most once and safely chains the prior hook.
pub fn install_panic_hook(store: DiagnosticStore) {
    if PANIC_STORE.set(RwLock::new(Arc::new(store))).is_err() {
        return;
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        PANIC_COUNT.fetch_add(1, Ordering::Relaxed);
        let reentered = IN_PANIC_HOOK.with(|active| active.replace(true));
        if reentered {
            eprintln!("parallel-world: recursive panic hook suppressed");
            return;
        }
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if let Some(store) = PANIC_STORE.get()
                && let Ok(store) = store.read()
            {
                let _ = store.write_unpruned(CrashInput::panic(info));
            }
        }));
        if let Some(sender) = MAINTENANCE_TX.get() {
            let _ = sender.try_send(());
        }
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| previous(info)));
        IN_PANIC_HOOK.with(|active| active.set(false));
    }));
}

#[must_use]
pub fn panic_count() -> u64 {
    PANIC_COUNT.load(Ordering::Relaxed)
}

/// Starts the bounded, coalescing retention worker once. Panic hooks only signal it.
/// # Errors
/// Returns an error if the dedicated maintenance thread cannot be spawned.
pub fn start_diagnostic_maintenance() -> io::Result<()> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::Builder::new()
        .name("diagnostic-retention".into())
        .spawn(move || {
            loop {
                if receiver.recv().is_err() {
                    break;
                }
                loop {
                    if let Some(store) = PANIC_STORE.get()
                        && let Ok(store) = store.read()
                    {
                        let _ = store.maintain();
                    }
                    match receiver.try_recv() {
                        Ok(()) => {}
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
                    }
                }
            }
        })?;
    if MAINTENANCE_TX.set(sender).is_err() {
        drop(worker);
    }
    Ok(())
}

pub fn configure_panic_store(store: DiagnosticStore) {
    if let Some(current) = PANIC_STORE.get()
        && let Ok(mut current) = current.write()
    {
        *current = Arc::new(store);
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
fn bounded(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}
fn redact(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    let markers = [
        "bearer ",
        "sk-",
        "api_key",
        "apikey",
        "password",
        "credential",
        "prompt=",
        "prompt:",
    ];
    if markers.iter().any(|marker| lower.contains(marker)) {
        "[redacted]".into()
    } else {
        bounded(value, 256)
    }
}
fn capture_backtrace() -> Option<String> {
    std::env::var_os("RUST_BACKTRACE")
        .filter(|value| value != "0")
        .map(|_| bounded(&Backtrace::force_capture().to_string(), 16 * 1024))
}

fn unique_temp(directory: &Path, prefix: &str) -> io::Result<(PathBuf, fs::File)> {
    for _ in 0..128 {
        let path = directory.join(format!(
            "{prefix}{}-{}-{}.tmp",
            now_ms(),
            std::process::id(),
            REPORT_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "unable to allocate unique diagnostic temp file",
    ))
}

#[cfg(windows)]
/// Atomically replaces a destination with a fully-written temporary file.
/// # Errors
/// Returns an OS I/O error when the replacement cannot be completed.
pub fn atomic_replace(source: &Path, destination: &Path) -> io::Result<()> {
    windows_atomic::replace(source, destination)
}
#[cfg(not(windows))]
/// Atomically replaces a destination with a fully-written temporary file.
/// # Errors
/// Returns an OS I/O error when the replacement cannot be completed.
pub fn atomic_replace(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

/// Removes oldest regular files until both retention limits are met.
/// # Errors
/// Returns an I/O error when the directory cannot be inspected or pruned.
pub fn prune_directory(directory: &Path, policy: RetentionPolicy) -> Result<(), DiagnosticError> {
    if !directory.exists() {
        return Ok(());
    }
    let mut files = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata.is_file().then_some((
                entry.path(),
                metadata.modified().unwrap_or(UNIX_EPOCH),
                metadata.len(),
            ))
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|(_, modified, _)| std::cmp::Reverse(*modified));
    let mut total: u64 = files.iter().map(|(_, _, bytes)| bytes).sum();
    while files.len() > policy.max_files || total > policy.max_bytes {
        if let Some((path, _, bytes)) = files.pop() {
            total = total.saturating_sub(bytes);
            fs::remove_file(path)?;
        } else {
            break;
        }
    }
    Ok(())
}
