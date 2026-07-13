use std::{
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
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        if probe_ready(probe) {
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
                        && !probe_ready(probe)
                        && child.try_wait().ok().flatten().is_none()
                        && std::time::Instant::now() < ready_deadline
                    {
                        thread::sleep(Duration::from_millis(100));
                    }
                    if !probe_ready(probe) {
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

fn probe_ready(address: SocketAddr) -> bool {
    TcpStream::connect_timeout(&address, Duration::from_millis(250)).is_ok()
}

fn sleep_interruptibly(stop: &AtomicBool, duration: Duration) {
    let mut remaining = duration;
    while !stop.load(Ordering::Acquire) && !remaining.is_zero() {
        let slice = remaining.min(Duration::from_millis(100));
        thread::sleep(slice);
        remaining = remaining.saturating_sub(slice);
    }
}
