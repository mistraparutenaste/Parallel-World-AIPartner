use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::mpsc::{self, Sender},
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use pw_platform::{diagnostics::atomic_replace, paths::AppDataLayout};
use serde::{Deserialize, Serialize};
use tauri::Manager;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StabilityHeartbeat {
    pub schema_version: u16,
    pub process_id: u32,
    pub run_id: String,
    pub started_timestamp_ms: u64,
    pub timestamp_ms: u64,
    pub audio_device: String,
    pub supervisor_healthy: bool,
    pub input_queue_depth: u64,
    pub output_queue_depth: u64,
    pub dropped_items: u64,
    pub cache_file_count: u64,
    pub log_bytes: u64,
    pub restart_count: u64,
    pub panic_count: u64,
    pub fault_count: u64,
    pub child_process_ids: Vec<u32>,
}

pub struct StabilityHeartbeatService {
    stop: Sender<()>,
    worker: std::sync::Mutex<Option<JoinHandle<()>>>,
}

impl StabilityHeartbeatService {
    /// Starts the one-second managed heartbeat writer.
    ///
    /// # Errors
    /// Returns an I/O error when the managed worker thread cannot be created.
    pub fn start(app: tauri::AppHandle, path: PathBuf) -> io::Result<Self> {
        let started_timestamp_ms = now_ms();
        let identity = HeartbeatIdentity {
            process_id: std::process::id(),
            run_id: format!("{}-{started_timestamp_ms}", std::process::id()),
            started_timestamp_ms,
        };
        let (stop, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("stability-heartbeat".into())
            .spawn(move || {
                loop {
                    let heartbeat = collect(&app, &identity);
                    if let Err(error) = write_atomic(&path, &heartbeat) {
                        tracing::warn!(%error, "failed to write stability heartbeat");
                    }
                    if receiver.recv_timeout(Duration::from_secs(1)).is_ok() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            stop,
            worker: std::sync::Mutex::new(Some(worker)),
        })
    }

    pub fn shutdown(&self) {
        let _ = self.stop.send(());
        if let Some(worker) = self
            .worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _ = worker.join();
        }
    }
}

struct HeartbeatIdentity {
    process_id: u32,
    run_id: String,
    started_timestamp_ms: u64,
}

impl Drop for StabilityHeartbeatService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn collect(app: &tauri::AppHandle, identity: &HeartbeatIdentity) -> StabilityHeartbeat {
    let chat = app.state::<crate::chat::ChatService>();
    let tts = app.state::<crate::tts::TtsService>();
    let speech = app.state::<crate::speech::SpeechService>();
    let processes = app
        .state::<crate::supervisor::ManagedProcesses>()
        .diagnostics();
    let layout = app.state::<AppDataLayout>();
    let chat_metrics = chat.queue_metrics();
    let tts_metrics = tts.queue_metrics();
    let audio = speech.diagnostics();
    StabilityHeartbeat {
        schema_version: 1,
        process_id: identity.process_id,
        run_id: identity.run_id.clone(),
        started_timestamp_ms: identity.started_timestamp_ms,
        timestamp_ms: now_ms(),
        audio_device: speech.active_device_id(),
        supervisor_healthy: processes.healthy,
        input_queue_depth: chat_metrics.iter().map(|metric| metric.depth as u64).sum(),
        output_queue_depth: tts_metrics.depth as u64,
        dropped_items: chat_metrics
            .iter()
            .map(|metric| metric.dropped)
            .sum::<u64>()
            + tts_metrics.dropped
            + audio.dropped_samples
            + audio.failure_queue_dropped,
        cache_file_count: file_stats(&layout.cache).0,
        log_bytes: file_stats(&layout.logs).1,
        restart_count: processes.restart_count,
        panic_count: pw_platform::diagnostics::panic_count(),
        fault_count: processes.fault_count,
        child_process_ids: processes.owned_child_pids,
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

fn file_stats(path: &Path) -> (u64, u64) {
    let Ok(entries) = fs::read_dir(path) else {
        return (0, 0);
    };
    entries.flatten().fold((0, 0), |(count, bytes), entry| {
        entry.metadata().map_or((count, bytes), |metadata| {
            if metadata.is_dir() {
                let nested = file_stats(&entry.path());
                (
                    count.saturating_add(nested.0),
                    bytes.saturating_add(nested.1),
                )
            } else if metadata.is_file() {
                (count + 1, bytes.saturating_add(metadata.len()))
            } else {
                (count, bytes)
            }
        })
    })
}

fn write_atomic(path: &Path, heartbeat: &StabilityHeartbeat) -> io::Result<()> {
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec(heartbeat)?)?;
    atomic_replace(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_is_schema_complete_and_atomically_replaced() {
        let directory = std::env::temp_dir().join(format!("pw-heartbeat-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("soak-heartbeat.json");
        let mut heartbeat = StabilityHeartbeat {
            schema_version: 1,
            process_id: 123,
            run_id: "123-test".into(),
            started_timestamp_ms: now_ms(),
            timestamp_ms: now_ms(),
            audio_device: "test-device".into(),
            supervisor_healthy: true,
            input_queue_depth: 2,
            output_queue_depth: 3,
            dropped_items: 4,
            cache_file_count: 5,
            log_bytes: 6,
            restart_count: 7,
            panic_count: 0,
            fault_count: 8,
            child_process_ids: vec![9],
        };
        write_atomic(&path, &heartbeat).unwrap();
        heartbeat.timestamp_ms += 1;
        write_atomic(&path, &heartbeat).unwrap();
        let decoded: StabilityHeartbeat =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(decoded, heartbeat);
        assert!(now_ms().saturating_sub(decoded.timestamp_ms) < 10_000);
        fs::remove_dir_all(directory).unwrap();
    }
}
