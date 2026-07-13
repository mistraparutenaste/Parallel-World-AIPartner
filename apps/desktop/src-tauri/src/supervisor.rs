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
use pw_contracts::{ProcessOwnershipDto, RUNTIME_HEALTH_EVENT, RuntimeHealthEventDto};
use pw_domain::runtime_health::{FailureCode, RuntimeFailure, RuntimeFeature, RuntimeHealth};
use pw_platform::process::{ProcessSpec, ProcessSupervisor};
use tauri::Emitter;

trait RuntimeHealthSink: Send + Sync {
    fn publish(&self, event: RuntimeHealthEventDto);
}

struct AppRuntimeHealthSink(tauri::AppHandle);
impl RuntimeHealthSink for AppRuntimeHealthSink {
    fn publish(&self, event: RuntimeHealthEventDto) {
        if let Err(error) = self.0.emit(RUNTIME_HEALTH_EVENT, event) {
            tracing::warn!(%error, "failed to emit runtime health event");
        }
    }
}

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
    lifecycle: Mutex<Lifecycle>,
    circuits: Arc<AtomicU64>,
    sink: Arc<dyn RuntimeHealthSink>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    Running,
    Shutdown,
}

#[derive(Clone)]
struct MonitorConfig {
    name: &'static str,
    spec: ProcessSpec,
    probe: SocketAddr,
    probe_paths: &'static [&'static str],
    feature: RuntimeFeature,
}

impl ManagedProcesses {
    #[must_use]
    pub fn from_environment(app: tauri::AppHandle) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let generation = Arc::new(AtomicU64::new(1));
        let mut configs = Vec::new();
        for (name, feature, variable, args_variable, port_variable, default_port) in [
            (
                "AivisSpeech",
                RuntimeFeature::TextToSpeech,
                "PW_AIVIS_ENGINE",
                "PW_AIVIS_ARGS_JSON",
                "PW_TTS_PORT",
                10101,
            ),
            (
                "llama-server",
                RuntimeFeature::LanguageModel,
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
                feature,
            });
        }
        let sink: Arc<dyn RuntimeHealthSink> = Arc::new(AppRuntimeHealthSink(app));
        let circuits = Arc::new(AtomicU64::new(0));
        let handles = spawn_monitors(&configs, &stop, &generation, &sink, &circuits);
        Self {
            stop,
            generation,
            configs,
            handles: Mutex::new(handles),
            lifecycle: Mutex::new(Lifecycle::Running),
            circuits,
            sink,
        }
    }

    pub fn shutdown(&self) {
        let mut lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *lifecycle == Lifecycle::Shutdown {
            return;
        }
        *lifecycle = Lifecycle::Shutdown;
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

    #[must_use]
    pub fn is_running(&self) -> bool {
        *self
            .lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            == Lifecycle::Running
    }

    /// Rearms a configured feature only when its circuit is open.
    ///
    /// # Errors
    /// Returns an error after terminal shutdown, for an unopened circuit, or an unconfigured feature.
    pub fn rearm(&self, feature: RuntimeFeature) -> Result<(), &'static str> {
        let lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *lifecycle != Lifecycle::Running {
            return Err("process supervision is shut down");
        }
        let bit = feature_bit(feature);
        let config = self
            .configs
            .iter()
            .find(|config| config.feature == feature)
            .cloned()
            .ok_or("feature is not configured")?;
        if self.circuits.fetch_and(!bit, Ordering::AcqRel) & bit == 0 {
            return Err("feature circuit is not open");
        }
        let handle = spawn_monitor(
            config.name,
            config.spec,
            config.probe,
            config.probe_paths,
            self.stop.clone(),
            self.generation.clone(),
            self.generation.load(Ordering::Acquire),
            config.feature,
            self.sink.clone(),
            self.circuits.clone(),
        );
        self.handles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(handle);
        Ok(())
    }
}

const fn feature_bit(feature: RuntimeFeature) -> u64 {
    match feature {
        RuntimeFeature::SpeechToText => 1,
        RuntimeFeature::LanguageModel => 2,
        RuntimeFeature::TextToSpeech => 4,
        RuntimeFeature::Live2D => 8,
        RuntimeFeature::AudioInput => 16,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MonitorDecision {
    Continue,
    Healthy,
    Restart,
}

#[derive(Default)]
struct MonitorCore {
    ready: bool,
    consecutive_probe_failures: u8,
}

impl MonitorCore {
    fn readiness(&mut self, healthy: bool) -> MonitorDecision {
        self.ready = healthy;
        if healthy {
            MonitorDecision::Healthy
        } else {
            MonitorDecision::Restart
        }
    }
    fn steady_probe(&mut self, healthy: bool) -> MonitorDecision {
        if healthy {
            self.consecutive_probe_failures = 0;
            return MonitorDecision::Continue;
        }
        self.consecutive_probe_failures = self.consecutive_probe_failures.saturating_add(1);
        if self.consecutive_probe_failures >= 3 {
            MonitorDecision::Restart
        } else {
            MonitorDecision::Continue
        }
    }
    const fn child_status(status: Result<bool, ()>) -> MonitorDecision {
        match status {
            Ok(true) => MonitorDecision::Continue,
            Ok(false) | Err(()) => MonitorDecision::Restart,
        }
    }
}

impl Drop for ManagedProcesses {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
/// Rearms a managed feature from the Settings window.
///
/// # Errors
/// Returns a safe reason when the feature cannot be rearmed.
pub fn rearm_managed_process(
    feature: pw_contracts::RuntimeFeatureDto,
    processes: tauri::State<'_, ManagedProcesses>,
) -> Result<(), String> {
    let feature = match feature {
        pw_contracts::RuntimeFeatureDto::LanguageModel => RuntimeFeature::LanguageModel,
        pw_contracts::RuntimeFeatureDto::TextToSpeech => RuntimeFeature::TextToSpeech,
        _ => return Err("feature is not managed by the process supervisor".into()),
    };
    processes.rearm(feature).map_err(str::to_owned)
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn spawn_monitor(
    name: &'static str,
    spec: ProcessSpec,
    probe: SocketAddr,
    probe_paths: &'static [&'static str],
    stop: Arc<AtomicBool>,
    generations: Arc<AtomicU64>,
    generation: u64,
    feature: RuntimeFeature,
    sink: Arc<dyn RuntimeHealthSink>,
    circuits: Arc<AtomicU64>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        if probe_ready(probe, probe_paths) {
            tracing::info!(process=name, %probe, "existing external process detected; not owned");
            monitor_external(
                name,
                feature,
                probe,
                probe_paths,
                &stop,
                &generations,
                generation,
                sink.as_ref(),
            );
            return;
        }
        let clock = SystemClock;
        let seed = clock.now_ms() ^ u64::from(std::process::id());
        let mut backoff = BackoffPolicy::new(&clock, Jitter(seed));
        while !stop.load(Ordering::Acquire) && generations.load(Ordering::Acquire) == generation {
            match ProcessSupervisor::spawn(&spec) {
                Ok(child) => {
                    let mut core = MonitorCore::default();
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
                        && matches!(child.try_wait(), Ok(None))
                        && std::time::Instant::now() < ready_deadline
                    {
                        thread::sleep(Duration::from_millis(100));
                    }
                    if core.readiness(probe_ready(probe, probe_paths)) == MonitorDecision::Restart {
                        tracing::warn!(process=name, %probe, "readiness probe failed");
                        let _ = child.stop(Duration::from_secs(5));
                    } else {
                        publish_health(
                            sink.as_ref(),
                            feature,
                            ProcessOwnershipDto::Managed,
                            true,
                            backoff.attempts(),
                            false,
                        );
                    }
                    let mut next_probe = std::time::Instant::now() + Duration::from_secs(1);
                    while !stop.load(Ordering::Acquire)
                        && generations.load(Ordering::Acquire) == generation
                    {
                        match child.try_wait() {
                            Ok(None) => {
                                debug_assert_eq!(
                                    MonitorCore::child_status(Ok(true)),
                                    MonitorDecision::Continue
                                );
                            }
                            Ok(Some(_)) => break,
                            Err(error) => {
                                tracing::error!(process=name, %error, "managed process status query failed");
                                publish_health(
                                    sink.as_ref(),
                                    feature,
                                    ProcessOwnershipDto::Managed,
                                    false,
                                    1,
                                    false,
                                );
                                let _ = child.stop(Duration::from_secs(5));
                                break;
                            }
                        }
                        if std::time::Instant::now() >= next_probe {
                            next_probe += Duration::from_secs(1);
                            let healthy = probe_ready(probe, probe_paths);
                            if core.steady_probe(healthy) == MonitorDecision::Restart {
                                publish_health(
                                    sink.as_ref(),
                                    feature,
                                    ProcessOwnershipDto::Managed,
                                    false,
                                    backoff.attempts().saturating_add(1),
                                    false,
                                );
                                let _ = child.stop(Duration::from_secs(5));
                                break;
                            }
                        }
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
            publish_health(
                sink.as_ref(),
                feature,
                ProcessOwnershipDto::Managed,
                false,
                backoff.attempts().saturating_add(1),
                false,
            );
            match backoff.record_failure() {
                BackoffDecision::RetryAfter(delay) => sleep_interruptibly(&stop, delay),
                BackoffDecision::CircuitOpen => {
                    tracing::error!(process = name, "managed process restart circuit opened");
                    publish_health(
                        sink.as_ref(),
                        feature,
                        ProcessOwnershipDto::Managed,
                        false,
                        8,
                        true,
                    );
                    circuits.fetch_or(feature_bit(feature), Ordering::Release);
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
    sink: &Arc<dyn RuntimeHealthSink>,
    circuits: &Arc<AtomicU64>,
) -> Vec<JoinHandle<()>> {
    spawn_monitors_at(
        configs,
        stop,
        generations,
        generations.load(Ordering::Acquire),
        sink,
        circuits,
    )
}

fn spawn_monitors_at(
    configs: &[MonitorConfig],
    stop: &Arc<AtomicBool>,
    generations: &Arc<AtomicU64>,
    generation: u64,
    sink: &Arc<dyn RuntimeHealthSink>,
    circuits: &Arc<AtomicU64>,
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
                config.feature,
                sink.clone(),
                circuits.clone(),
            )
        })
        .collect()
}

fn publish_health(
    sink: &dyn RuntimeHealthSink,
    feature: RuntimeFeature,
    ownership: ProcessOwnershipDto,
    healthy: bool,
    attempts: u8,
    circuit_open: bool,
) {
    let mut health = RuntimeHealth::new(feature);
    if healthy {
        health.mark_healthy(SystemClock.now_ms());
    } else {
        health.mark_failed(
            &RuntimeFailure::transient(FailureCode::Unavailable),
            SystemClock.now_ms(),
        );
    }
    let mut event = RuntimeHealthEventDto::from((&health, attempts));
    event.ownership = ownership;
    event.circuit_open = circuit_open;
    sink.publish(event);
}

#[allow(clippy::too_many_arguments)]
fn monitor_external(
    name: &'static str,
    feature: RuntimeFeature,
    probe: SocketAddr,
    paths: &'static [&'static str],
    stop: &AtomicBool,
    generations: &AtomicU64,
    generation: u64,
    sink: &dyn RuntimeHealthSink,
) {
    let mut failures = 0_u8;
    publish_health(sink, feature, ProcessOwnershipDto::External, true, 0, false);
    while !stop.load(Ordering::Acquire) && generations.load(Ordering::Acquire) == generation {
        thread::sleep(Duration::from_secs(1));
        if probe_ready(probe, paths) {
            failures = 0;
            publish_health(sink, feature, ProcessOwnershipDto::External, true, 0, false);
        } else {
            failures = failures.saturating_add(1);
            if failures >= 3 {
                tracing::warn!(process = name, "external process probe threshold exceeded");
                publish_health(
                    sink,
                    feature,
                    ProcessOwnershipDto::External,
                    false,
                    failures,
                    false,
                );
            }
        }
    }
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

    #[test]
    fn core_does_not_report_healthy_before_readiness() {
        let core = MonitorCore::default();
        assert!(!core.ready);
    }
    #[test]
    fn core_reports_healthy_after_successful_readiness() {
        let mut core = MonitorCore::default();
        assert_eq!(core.readiness(true), MonitorDecision::Healthy);
    }
    #[test]
    fn core_restarts_after_failed_readiness() {
        let mut core = MonitorCore::default();
        assert_eq!(core.readiness(false), MonitorDecision::Restart);
    }
    #[test]
    fn core_tolerates_two_steady_probe_failures() {
        let mut core = MonitorCore::default();
        assert_eq!(core.steady_probe(false), MonitorDecision::Continue);
        assert_eq!(core.steady_probe(false), MonitorDecision::Continue);
    }
    #[test]
    fn core_restarts_on_third_steady_probe_failure() {
        let mut core = MonitorCore::default();
        core.steady_probe(false);
        core.steady_probe(false);
        assert_eq!(core.steady_probe(false), MonitorDecision::Restart);
    }
    #[test]
    fn core_success_resets_probe_failure_threshold() {
        let mut core = MonitorCore::default();
        core.steady_probe(false);
        core.steady_probe(false);
        core.steady_probe(true);
        assert_eq!(core.steady_probe(false), MonitorDecision::Continue);
    }
    #[test]
    fn core_restarts_when_child_exits() {
        assert_eq!(
            MonitorCore::child_status(Ok(false)),
            MonitorDecision::Restart
        );
    }
    #[test]
    fn core_restarts_when_child_status_query_fails() {
        assert_eq!(MonitorCore::child_status(Err(())), MonitorDecision::Restart);
    }

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
