//! Redacted, bounded crash diagnostics persisted with atomic replacement.

use serde::{Deserialize, Serialize};
#[cfg(windows)]
mod windows_atomic;
use std::{
    backtrace::Backtrace,
    cell::Cell,
    fs, io,
    io::{Read, Write},
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
const REPORT_SCHEMA_VERSION: u32 = 1;
const MAX_REPORT_BYTES: u64 = 256 * 1024;
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CrashReport {
    schema_version: u32,
    timestamp_ms: u64,
    build: String,
    source: String,
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
            schema_version: REPORT_SCHEMA_VERSION,
            timestamp_ms,
            build: env!("CARGO_PKG_VERSION").into(),
            source: input.source.into(),
            payload_category: input.category,
            detail: input.detail,
            thread: input.thread,
            location: input.location,
            backtrace: input.backtrace,
        };
        if !report.is_valid(timestamp_ms) {
            return Err(DiagnosticError::InvalidReport);
        }
        let bytes = serde_json::to_vec_pretty(&report)?;
        let mut temporary = unique_temp(&self.directory, ".pw-crash-")?;
        temporary.file_mut()?.write_all(&bytes)?;
        temporary.file_mut()?.sync_all()?;
        temporary.close();
        if let Err(error) = fs::rename(temporary.path()?, &destination) {
            return Err(error.into());
        }
        temporary.persisted();
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
            let Some(report) = read_valid_report(&path, &id)? else {
                continue;
            };
            entries.push(DiagnosticEntry {
                schema_version: 1,
                id,
                timestamp_ms: report.timestamp_ms,
                category: report.payload_category,
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
        self.remove_invalid_json()?;
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

    fn remove_invalid_json(&self) -> Result<(), DiagnosticError> {
        if !self.directory.exists() {
            return Ok(());
        }
        for item in fs::read_dir(&self.directory)? {
            let item = item?;
            let path = item.path();
            if item.file_type()?.is_file()
                && path.extension().and_then(|value| value.to_str()) == Some("json")
            {
                let id = item.file_name().to_string_lossy().into_owned();
                if read_valid_report(&path, &id)?.is_none() {
                    fs::remove_file(path)?;
                }
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
            let path = self.directory.join(&entry.id);
            if let Some(report) = read_valid_report(&path, &entry.id)? {
                reports.push(report);
            }
        }
        let mut temporary = unique_temp(&parent, ".pw-export-")?;
        temporary
            .file_mut()?
            .write_all(&serde_json::to_vec_pretty(&reports)?)?;
        temporary.file_mut()?.sync_all()?;
        temporary.close();
        let result = if allow_overwrite {
            atomic_replace(temporary.path()?, destination)
        } else {
            fs::hard_link(temporary.path()?, destination)
                .and_then(|()| fs::remove_file(temporary.path()?))
        };
        if let Err(error) = result {
            return Err(error.into());
        }
        temporary.persisted();
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
    #[error("diagnostic report violates the safe schema")]
    InvalidReport,
}

static PANIC_STORE: OnceLock<RwLock<Arc<DiagnosticStore>>> = OnceLock::new();
static MAINTENANCE_TX: OnceLock<std::sync::mpsc::SyncSender<()>> = OnceLock::new();
thread_local! { static IN_PANIC_HOOK: Cell<bool> = const { Cell::new(false) }; }

/// Installs the process panic hook at most once.
///
/// The previous hook is deliberately replaced rather than called: Rust aborts if a
/// hook panics while handling a panic, so an arbitrary prior hook cannot be made safe
/// with `catch_unwind`.
pub fn install_panic_hook(store: DiagnosticStore) {
    if PANIC_STORE.set(RwLock::new(Arc::new(store))).is_err() {
        return;
    }
    std::panic::set_hook(Box::new(move |info| {
        PANIC_COUNT.fetch_add(1, Ordering::Relaxed);
        let reentered = IN_PANIC_HOOK
            .try_with(|active| active.replace(true))
            .unwrap_or(true);
        if reentered {
            return;
        }
        if let Some(store) = PANIC_STORE.get()
            && let Ok(store) = store.try_read()
        {
            let _ = store.write_unpruned(CrashInput::panic(info));
        }
        if let Some(sender) = MAINTENANCE_TX.get() {
            let _ = sender.try_send(());
        }
        let _ = IN_PANIC_HOOK.try_with(|active| active.set(false));
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
    redact_bounded(value, 256)
}
fn redact_bounded(value: &str, max: usize) -> String {
    if contains_sensitive(value) {
        "[redacted]".into()
    } else {
        bounded(value, max)
    }
}
fn contains_sensitive(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let markers = [
        "bearer ",
        "sk-",
        "api_key",
        "apikey",
        "password",
        "credential",
        "authorization",
        "access_token",
        "refresh_token",
        "prompt",
    ];
    markers.iter().any(|marker| lower.contains(marker))
}
fn capture_backtrace() -> Option<String> {
    std::env::var_os("RUST_BACKTRACE")
        .filter(|value| value != "0")
        .map(|_| redact_bounded(&Backtrace::force_capture().to_string(), 16 * 1024))
}

struct AtomicTemp {
    path: Option<PathBuf>,
    file: Option<fs::File>,
}

impl AtomicTemp {
    fn path(&self) -> io::Result<&Path> {
        self.path.as_deref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "temporary file is no longer available",
            )
        })
    }

    fn file_mut(&mut self) -> io::Result<&mut fs::File> {
        self.file.as_mut().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "temporary file is already closed")
        })
    }

    fn close(&mut self) {
        self.file.take();
    }

    fn persisted(&mut self) {
        self.path.take();
    }
}

impl Drop for AtomicTemp {
    fn drop(&mut self) {
        self.file.take();
        if let Some(path) = self.path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

fn unique_temp(directory: &Path, prefix: &str) -> io::Result<AtomicTemp> {
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
            Ok(file) => {
                return Ok(AtomicTemp {
                    path: Some(path),
                    file: Some(file),
                });
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "unable to allocate unique diagnostic temp file",
    ))
}

fn read_valid_report(path: &Path, id: &str) -> Result<Option<CrashReport>, DiagnosticError> {
    let Some((timestamp_ms, _, _)) = parse_report_name(id) else {
        return Ok(None);
    };
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_REPORT_BYTES {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_REPORT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_REPORT_BYTES {
        return Ok(None);
    }
    let Ok(report) = serde_json::from_slice::<CrashReport>(&bytes) else {
        return Ok(None);
    };
    if report.is_valid(timestamp_ms) {
        Ok(Some(report))
    } else {
        Ok(None)
    }
}

fn parse_report_name(id: &str) -> Option<(u64, u32, u64)> {
    let stem = id.strip_prefix("crash-")?.strip_suffix(".json")?;
    let mut parts = stem.split('-');
    let timestamp_ms = parts.next()?.parse().ok()?;
    let process_id = parts.next()?.parse().ok()?;
    let sequence = parts.next()?.parse().ok()?;
    parts
        .next()
        .is_none()
        .then_some((timestamp_ms, process_id, sequence))
}

impl CrashReport {
    fn is_valid(&self, filename_timestamp_ms: u64) -> bool {
        if self.schema_version != REPORT_SCHEMA_VERSION
            || self.timestamp_ms != filename_timestamp_ms
            || !safe_metadata(&self.build, 128)
            || self
                .thread
                .as_deref()
                .is_some_and(|value| !safe_metadata(value, 256))
            || self
                .location
                .as_deref()
                .is_some_and(|value| !safe_metadata(value, 256))
            || self
                .backtrace
                .as_deref()
                .is_some_and(|value| !safe_metadata(value, 16 * 1024))
        {
            return false;
        }
        match (self.source.as_str(), self.payload_category.as_str()) {
            ("rust", "panic_str" | "panic_string" | "panic_non_string") => {
                self.detail == "panic payload omitted"
            }
            ("frontend", "window_error" | "unhandled_rejection") => {
                valid_frontend_detail(&self.detail)
                    && self.thread.is_none()
                    && self.location.is_none()
                    && self.backtrace.is_none()
            }
            _ => false,
        }
    }
}

fn safe_metadata(value: &str, max: usize) -> bool {
    value.chars().count() <= max && !contains_sensitive(value)
}

fn valid_frontend_detail(value: &str) -> bool {
    let Some((line, column)) = value
        .strip_prefix("line=")
        .and_then(|value| value.split_once(";column="))
    else {
        return false;
    };
    line.parse::<u32>().is_ok() && column.parse::<u32>().is_ok()
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

#[cfg(test)]
mod tests {
    use super::unique_temp;

    #[test]
    fn dropping_an_uncommitted_temp_removes_it() {
        let root =
            std::env::temp_dir().join(format!("pw-diagnostics-temp-drop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let temporary = unique_temp(&root, ".pw-crash-").unwrap();
        let path = temporary.path().unwrap().to_path_buf();

        drop(temporary);

        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
