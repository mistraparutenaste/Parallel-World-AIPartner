//! Startup wiring: app data directories and logging.

use pw_platform::diagnostics::{
    DiagnosticStore, RetentionPolicy, configure_panic_store, prune_directory,
};
use pw_platform::paths::AppDataLayout;
use std::{
    fs,
    io::{BufRead, Read, Seek, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::{AppHandle, Manager, Runtime};

use crate::error::AppError;

const LOG_FILE_LIMIT: u64 = 5 * 1024 * 1024;
const LOG_READ_LIMIT: u64 = 128 * 1024;
const LOG_LINE_LIMIT: usize = 16 * 1024;

fn truncate_utf8_bytes(value: &str) -> String {
    if value.len() <= LOG_LINE_LIMIT {
        return value.to_owned();
    }
    let mut end = LOG_LINE_LIMIT;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}
#[derive(Clone)]
struct BoundedLogMakeWriter {
    state: Arc<Mutex<BoundedLogState>>,
}
struct BoundedLogState {
    directory: PathBuf,
    sequence: u64,
    generation: u64,
}

#[derive(Clone)]
pub struct TechnicalLogAccess {
    state: Arc<Mutex<BoundedLogState>>,
}

impl TechnicalLogAccess {
    fn new(state: Arc<Mutex<BoundedLogState>>) -> Self {
        Self { state }
    }

    /// Reads a bounded chunk from the current application log.
    ///
    /// # Errors
    ///
    /// Returns an error when the current log cannot be inspected or read.
    pub fn read(
        &self,
        cursor: Option<pw_contracts::TechnicalLogCursorDto>,
    ) -> Result<pw_contracts::TechnicalLogChunkDto, String> {
        let (path, generation) = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (state.directory.join("parallel-world.log"), state.generation)
        };
        let mut file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(pw_contracts::TechnicalLogChunkDto {
                    schema_version: pw_contracts::SCHEMA_VERSION,
                    lines: Vec::new(),
                    next_cursor: pw_contracts::TechnicalLogCursorDto {
                        generation,
                        offset: 0,
                    },
                    reset: cursor.is_some_and(|value| value.generation != generation),
                    has_more: false,
                });
            }
            Err(error) => return Err(error.to_string()),
        };
        let length = file.metadata().map_err(|error| error.to_string())?.len();
        let reset =
            cursor.is_some_and(|value| value.generation != generation || value.offset > length);
        let initial = cursor.is_none() || reset;
        let offset = if initial {
            length.saturating_sub(LOG_READ_LIMIT)
        } else {
            cursor.map_or(0, |value| value.offset)
        };
        let starts_mid_line = if offset == 0 {
            false
        } else {
            file.seek(std::io::SeekFrom::Start(offset - 1))
                .map_err(|error| error.to_string())?;
            let mut previous = [0_u8; 1];
            file.read_exact(&mut previous)
                .map_err(|error| error.to_string())?;
            previous[0] != b'\n'
        };
        file.seek(std::io::SeekFrom::Start(offset))
            .map_err(|error| error.to_string())?;
        let mut bytes = Vec::new();
        (&mut file)
            .take(LOG_READ_LIMIT)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if u64::try_from(bytes.len()).unwrap_or(0) == LOG_READ_LIMIT && !bytes.ends_with(b"\n") {
            std::io::BufReader::new(&mut file)
                .read_until(b'\n', &mut bytes)
                .map_err(|error| error.to_string())?;
        }
        let complete_len = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        let complete = &bytes[..complete_len];
        let start = if starts_mid_line {
            complete
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(complete.len(), |index| index + 1)
        } else {
            0
        };
        let text = String::from_utf8_lossy(&complete[start..]);
        let lines = text
            .lines()
            .map(pw_domain::runtime_health::redact_credentials)
            .map(|line| truncate_utf8_bytes(&line))
            .collect();
        let next_offset = offset + u64::try_from(complete_len).unwrap_or(LOG_READ_LIMIT);
        Ok(pw_contracts::TechnicalLogChunkDto {
            schema_version: pw_contracts::SCHEMA_VERSION,
            lines,
            next_cursor: pw_contracts::TechnicalLogCursorDto {
                generation,
                offset: next_offset,
            },
            reset,
            has_more: next_offset < length && next_offset > offset,
        })
    }
}
struct BoundedLogWriter {
    state: Arc<Mutex<BoundedLogState>>,
}
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BoundedLogMakeWriter {
    type Writer = BoundedLogWriter;
    fn make_writer(&'a self) -> Self::Writer {
        BoundedLogWriter {
            state: Arc::clone(&self.state),
        }
    }
}
impl Write for BoundedLogWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let active = state.directory.join("parallel-world.log");
        let mut length = fs::metadata(&active).map_or(0, |metadata| metadata.len());
        if length >= LOG_FILE_LIMIT {
            let rotated = state.directory.join(format!(
                "parallel-world-{}-{}-{}.log",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis(),
                std::process::id(),
                state.sequence
            ));
            state.sequence = state.sequence.wrapping_add(1);
            fs::rename(&active, rotated)?;
            length = 0;
            state.generation = state.generation.wrapping_add(1);
        }
        let remaining =
            usize::try_from(LOG_FILE_LIMIT.saturating_sub(length)).unwrap_or(usize::MAX);
        let segment = &buffer[..buffer.len().min(remaining)];
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&active)?;
        file.write_all(segment)?;
        file.flush()?;
        drop(file);
        prune_closed_logs(&state.directory, &active)?;
        Ok(segment.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn prune_closed_logs(directory: &std::path::Path, active: &std::path::Path) -> std::io::Result<()> {
    let active_bytes = fs::metadata(active).map_or(0, |metadata| metadata.len());
    let mut closed = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if path == active {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            metadata.is_file().then_some((
                path,
                metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
                metadata.len(),
            ))
        })
        .collect::<Vec<_>>();
    closed.sort_by_key(|(_, modified, _)| std::cmp::Reverse(*modified));
    let policy = RetentionPolicy::default();
    let mut total = active_bytes + closed.iter().map(|(_, _, bytes)| bytes).sum::<u64>();
    while closed.len() + usize::from(active_bytes > 0) > policy.max_files
        || total > policy.max_bytes
    {
        if let Some((path, _, bytes)) = closed.pop() {
            fs::remove_file(path)?;
            total = total.saturating_sub(bytes);
        } else {
            break;
        }
    }
    Ok(())
}
struct LogRetentionGuard {
    stop: std::sync::mpsc::Sender<()>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for LogRetentionGuard {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Resolves the app data layout, creates all runtime directories and
/// installs a daily-rotating file logger under `logs/`.
///
/// Log output must never include API keys or environment variable
/// values.
///
/// # Errors
///
/// Returns [`AppError`] when the app data directory cannot be resolved
/// or created.
pub fn initialize<R: Runtime>(app: &AppHandle<R>) -> Result<AppDataLayout, AppError> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(AppError::AppDataDirUnavailable)?;
    let layout = AppDataLayout::under(root);
    layout.create_all()?;
    let crash_store = DiagnosticStore::new(&layout.crashes, RetentionPolicy::default());
    crash_store
        .recover_after_unclean_shutdown()
        .map_err(std::io::Error::other)?;
    configure_panic_store(crash_store.clone());
    prune_directory(&layout.logs, RetentionPolicy::default()).map_err(std::io::Error::other)?;

    let log_state = Arc::new(Mutex::new(BoundedLogState {
        directory: layout.logs.clone(),
        sequence: 0,
        generation: 0,
    }));
    let writer = BoundedLogMakeWriter {
        state: Arc::clone(&log_state),
    };
    app.manage(TechnicalLogAccess::new(log_state));
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(writer)
        .with_ansi(false)
        .init();
    let (stop, worker_stop) = std::sync::mpsc::channel();
    let logs = layout.logs.clone();
    let worker = std::thread::Builder::new()
        .name("log-retention".into())
        .spawn(move || {
            while worker_stop
                .recv_timeout(std::time::Duration::from_mins(1))
                .is_err()
            {
                let _ = prune_closed_logs(&logs, &logs.join("parallel-world.log"));
            }
        })?;
    app.manage(LogRetentionGuard {
        stop,
        worker: Some(worker),
    });

    tracing::info!(root = %layout.root.display(), "app data initialized");
    Ok(layout)
}

#[cfg(test)]
mod tests {
    use super::{
        BoundedLogMakeWriter, BoundedLogState, LOG_FILE_LIMIT, LOG_READ_LIMIT, TechnicalLogAccess,
    };
    use pw_contracts::TechnicalLogCursorDto;
    use std::{
        io::Write,
        sync::{Arc, Mutex},
    };
    use tracing_subscriber::fmt::MakeWriter;

    #[test]
    fn technical_log_reads_deltas_redacts_and_resets_after_rotation() {
        let root = std::env::temp_dir().join(format!("pw-log-read-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let state = Arc::new(Mutex::new(BoundedLogState {
            directory: root.clone(),
            sequence: 0,
            generation: 0,
        }));
        let make = BoundedLogMakeWriter {
            state: Arc::clone(&state),
        };
        let access = TechnicalLogAccess::new(state);
        let long = "x".repeat(20 * 1024);
        make.make_writer()
            .write_all(format!("token=abc\n{long}\n").as_bytes())
            .unwrap();

        let first = access.read(None).unwrap();
        assert!(first.lines.iter().any(|line| line.contains("[REDACTED]")));
        assert!(!first.lines.iter().any(|line| line.contains("abc")));
        assert!(first.lines.iter().all(|line| line.len() <= 16 * 1024));

        make.make_writer().write_all(b"next-line\n").unwrap();
        let delta = access.read(Some(first.next_cursor)).unwrap();
        assert_eq!(delta.lines, ["next-line"]);
        assert!(!delta.reset);

        std::fs::write(
            root.join("parallel-world.log"),
            vec![b'x'; usize::try_from(LOG_FILE_LIMIT).expect("log limit fits usize")],
        )
        .unwrap();
        make.make_writer().write_all(b"fresh-generation\n").unwrap();
        let reset = access.read(Some(delta.next_cursor)).unwrap();
        assert!(reset.reset);
        assert_eq!(reset.lines, ["fresh-generation"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn technical_log_redacts_across_chunk_boundaries_and_limits_utf8_bytes() {
        let root = std::env::temp_dir().join(format!("pw-log-boundary-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let state = Arc::new(Mutex::new(BoundedLogState {
            directory: root.clone(),
            sequence: 0,
            generation: 0,
        }));
        let access = TechnicalLogAccess::new(state);
        let mut bytes = vec![b'x'; usize::try_from(LOG_READ_LIMIT).unwrap() - b"token=".len()];
        bytes.extend_from_slice(b"token=abc\n");
        bytes.extend_from_slice("あ".repeat(10_000).as_bytes());
        bytes.push(b'\n');
        std::fs::write(root.join("parallel-world.log"), bytes).unwrap();

        let first = access
            .read(Some(TechnicalLogCursorDto {
                generation: 0,
                offset: 0,
            }))
            .unwrap();
        assert!(!first.lines.iter().any(|line| line.contains("abc")));
        assert!(first.lines.iter().all(|line| line.len() <= 16 * 1024));

        let second = access.read(Some(first.next_cursor)).unwrap();
        assert!(!second.lines.iter().any(|line| line.contains("abc")));
        assert!(second.lines.iter().all(|line| line.len() <= 16 * 1024));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn writer_rotates_before_active_log_exceeds_limit() {
        let root = std::env::temp_dir().join(format!("pw-log-rotate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let make = BoundedLogMakeWriter {
            state: Arc::new(Mutex::new(BoundedLogState {
                directory: root.clone(),
                sequence: 0,
                generation: 0,
            })),
        };
        let chunk = vec![b'x'; (usize::try_from(LOG_FILE_LIMIT).unwrap() / 2) + 1];
        make.make_writer().write_all(&chunk).unwrap();
        make.make_writer().write_all(&chunk).unwrap();
        assert!(
            std::fs::metadata(root.join("parallel-world.log"))
                .unwrap()
                .len()
                <= LOG_FILE_LIMIT
        );
        assert!(
            std::fs::read_dir(&root)
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("parallel-world-"))
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn huge_write_is_split_and_total_is_bounded_immediately() {
        let root = std::env::temp_dir().join(format!("pw-log-huge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let make = BoundedLogMakeWriter {
            state: Arc::new(Mutex::new(BoundedLogState {
                directory: root.clone(),
                sequence: 0,
                generation: 0,
            })),
        };
        let huge = vec![b'x'; usize::try_from(LOG_FILE_LIMIT * 6).unwrap()];
        make.make_writer().write_all(&huge).unwrap();
        let files = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        assert!(
            files
                .iter()
                .all(|entry| entry.metadata().unwrap().len() <= LOG_FILE_LIMIT)
        );
        assert!(
            files
                .iter()
                .map(|entry| entry.metadata().unwrap().len())
                .sum::<u64>()
                <= 20 * 1024 * 1024
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
