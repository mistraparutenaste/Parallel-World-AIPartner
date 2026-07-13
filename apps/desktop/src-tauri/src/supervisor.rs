use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
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
    generation: Arc<AtomicU64>,
    configs: Vec<MonitorConfig>,
    handles: Mutex<Vec<JoinHandle<()>>>,
}

#[derive(Clone)]
struct MonitorConfig {
    name: &'static str,
    spec: ProcessSpec,
    probe: SocketAddr,
    probe_paths: &'static [&'static str],
}

impl ManagedProcesses {
    #[must_use]
    pub fn from_environment() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let generation = Arc::new(AtomicU64::new(1));
        let mut configs = Vec::new();
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
            configs.push(MonitorConfig {
                name,
                spec: ProcessSpec {
                    executable: PathBuf::from(path),
                    args,
                    env: Vec::new(),
                    current_dir: None,
                    output_capacity: 64 * 1024,
                },
                probe: SocketAddr::from(([127, 0, 0, 1], port)),
                probe_paths,
            });
        }
        let handles = spawn_monitors(&configs, &stop, &generation);
        Self {
            stop,
            generation,
            configs,
            handles: Mutex::new(handles),
        }
    }

    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Release);
        self.generation.fetch_add(1, Ordering::AcqRel);
        for handle in self
            .handles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain(..)
        {
            let _ = handle.join();
        }
    }

    /// Explicitly rearms all configured monitors after stopping the current generation.
    pub fn restart(&self) {
        self.shutdown();
        self.stop.store(false, Ordering::Release);
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        *self
            .handles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            spawn_monitors_at(&self.configs, &self.stop, &self.generation, generation);
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
    generations: Arc<AtomicU64>,
    generation: u64,
) -> JoinHandle<()> {
    thread::spawn(move || {
        if probe_ready(probe, probe_paths) {
            tracing::info!(process=name, %probe, "existing external process detected; not owned");
            return;
        }
        let clock = SystemClock;
        let seed = clock.now_ms() ^ u64::from(std::process::id());
        let mut backoff = BackoffPolicy::new(&clock, Jitter(seed));
        while !stop.load(Ordering::Acquire) && generations.load(Ordering::Acquire) == generation {
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
                        && generations.load(Ordering::Acquire) == generation
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
                    while !stop.load(Ordering::Acquire)
                        && generations.load(Ordering::Acquire) == generation
                        && child.try_wait().ok().flatten().is_none()
                    {
                        thread::sleep(Duration::from_millis(100));
                        backoff.record_healthy();
                        let _ = backoff.reset_if_stable();
                    }
                    if stop.load(Ordering::Acquire)
                        || generations.load(Ordering::Acquire) != generation
                    {
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

fn spawn_monitors(
    configs: &[MonitorConfig],
    stop: &Arc<AtomicBool>,
    generations: &Arc<AtomicU64>,
) -> Vec<JoinHandle<()>> {
    spawn_monitors_at(
        configs,
        stop,
        generations,
        generations.load(Ordering::Acquire),
    )
}

fn spawn_monitors_at(
    configs: &[MonitorConfig],
    stop: &Arc<AtomicBool>,
    generations: &Arc<AtomicU64>,
    generation: u64,
) -> Vec<JoinHandle<()>> {
    configs
        .iter()
        .map(|config| {
            spawn_monitor(
                config.name,
                config.spec.clone(),
                config.probe,
                config.probe_paths,
                stop.clone(),
                generations.clone(),
                generation,
            )
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn serve_once(response: &'static [u8]) -> SocketAddr {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 256];
            let _ = stream.read(&mut request);
            stream.write_all(response).unwrap();
        });
        address
    }

    #[test]
    fn http_probe_accepts_only_success_status() {
        let ok = serve_once(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n");
        assert!(probe_http(ok, "/health"));
        let unavailable = serve_once(b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\n\r\n");
        assert!(!probe_http(unavailable, "/health"));
    }

    #[test]
    fn readiness_tries_fallback_path() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            for status in [404, 200] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 256];
                let _ = stream.read(&mut request);
                let response = format!("HTTP/1.1 {status} Test\r\nContent-Length: 0\r\n\r\n");
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        assert!(probe_ready(address, &["/health", "/v1/models"]));
    }
}
