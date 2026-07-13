use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use pw_application::recovery::{BackoffDecision, BackoffPolicy, Clock, RandomSource};
use pw_platform::process::{ProcessSpec, ProcessSupervisor};

struct SystemClock;
impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }
}
struct Jitter(u64);
impl RandomSource for Jitter {
    fn uniform_inclusive(&mut self, upper: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        if upper == u64::MAX {
            self.0
        } else {
            self.0 % (upper + 1)
        }
    }
}

pub struct ManagedProcesses {
    stop: Arc<AtomicBool>,
    handles: Mutex<Vec<JoinHandle<()>>>,
}

impl ManagedProcesses {
    #[must_use]
    pub fn from_environment() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let mut handles = Vec::new();
        for (name, variable, args_variable, port_variable, default_port) in [
            (
                "AivisSpeech",
                "PW_AIVIS_ENGINE",
                "PW_AIVIS_ARGS_JSON",
                "PW_TTS_PORT",
                10101,
            ),
            (
                "llama-server",
                "PW_LLAMA_SERVER",
                "PW_LLAMA_ARGS_JSON",
                "PW_LLM_PORT",
                8080,
            ),
        ] {
            let Some(path) = std::env::var_os(variable).filter(|value| !value.is_empty()) else {
                continue;
            };
            let args = match std::env::var(args_variable) {
                Ok(value) => match serde_json::from_str::<Vec<String>>(&value) {
                    Ok(args) => args.into_iter().map(Into::into).collect(),
                    Err(error) => {
                        tracing::error!(process=name, %error, "invalid JSON argument array");
                        continue;
                    }
                },
                Err(_) => Vec::new(),
            };
            let port = std::env::var(port_variable)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_port);
            let probe_paths: &'static [&'static str] = if name == "AivisSpeech" {
                &["/version", "/speakers"]
            } else {
                &["/health", "/v1/models"]
            };
            handles.push(spawn_monitor(
                name,
                ProcessSpec {
                    executable: PathBuf::from(path),
                    args,
                    env: Vec::new(),
                    current_dir: None,
                    output_capacity: 64 * 1024,
                },
                SocketAddr::from(([127, 0, 0, 1], port)),
                probe_paths,
                stop.clone(),
            ));
        }
        Self {
            stop,
            handles: Mutex::new(handles),
        }
    }

    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Release);
        for handle in self
            .handles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain(..)
        {
            let _ = handle.join();
        }
    }
}

impl Drop for ManagedProcesses {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn spawn_monitor(
    name: &'static str,
    spec: ProcessSpec,
    probe: SocketAddr,
    probe_paths: &'static [&'static str],
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        if probe_ready(probe, probe_paths) {
            tracing::info!(process=name, %probe, "existing external process detected; not owned");
            return;
        }
        let clock = SystemClock;
        let seed = clock.now_ms() ^ u64::from(std::process::id());
        let mut backoff = BackoffPolicy::new(&clock, Jitter(seed));
        while !stop.load(Ordering::Acquire) {
            match ProcessSupervisor::spawn(&spec) {
                Ok(child) => {
                    tracing::info!(
                        process = name,
                        pid = child.pid(),
                        generation = child.generation(),
                        "managed process started"
                    );
                    let ready_deadline = std::time::Instant::now() + Duration::from_secs(15);
                    while !stop.load(Ordering::Acquire)
                        && !probe_ready(probe, probe_paths)
                        && child.try_wait().ok().flatten().is_none()
                        && std::time::Instant::now() < ready_deadline
                    {
                        thread::sleep(Duration::from_millis(100));
                    }
                    if !probe_ready(probe, probe_paths) {
                        tracing::warn!(process=name, %probe, "readiness probe failed");
                        let _ = child.stop(Duration::from_secs(5));
                    }
                    while !stop.load(Ordering::Acquire) && child.try_wait().ok().flatten().is_none()
                    {
                        thread::sleep(Duration::from_millis(100));
                        backoff.record_healthy();
                        let _ = backoff.reset_if_stable();
                    }
                    if stop.load(Ordering::Acquire) {
                        let _ = child.stop(Duration::from_secs(5));
                        break;
                    }
                    tracing::warn!(process = name, "managed process exited unexpectedly");
                }
                Err(error) => {
                    tracing::warn!(process = name, %error, "managed process spawn failed");
                }
            }
            match backoff.record_failure() {
                BackoffDecision::RetryAfter(delay) => sleep_interruptibly(&stop, delay),
                BackoffDecision::CircuitOpen => {
                    tracing::error!(process = name, "managed process restart circuit opened");
                    break;
                }
            }
        }
    })
}

fn probe_ready(address: SocketAddr, paths: &[&str]) -> bool {
    paths.iter().any(|path| probe_http(address, path))
}

fn probe_http(address: SocketAddr, path: &str) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(250)) else {
        return false;
    };
    let timeout = Some(Duration::from_millis(500));
    if stream.set_read_timeout(timeout).is_err() || stream.set_write_timeout(timeout).is_err() {
        return false;
    }
    let request = format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut response = [0_u8; 64];
    let Ok(read) = stream.read(&mut response) else {
        return false;
    };
    let status = std::str::from_utf8(&response[..read]).unwrap_or_default();
    status.starts_with("HTTP/1.1 2") || status.starts_with("HTTP/1.0 2")
}

fn sleep_interruptibly(stop: &AtomicBool, duration: Duration) {
    let mut remaining = duration;
    while !stop.load(Ordering::Acquire) && !remaining.is_zero() {
        let slice = remaining.min(Duration::from_millis(100));
        thread::sleep(slice);
        remaining = remaining.saturating_sub(slice);
    }
}
