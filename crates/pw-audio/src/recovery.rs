//! Allocation-free stream failure notification and control-thread recovery policy.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioStreamFailure {
    DeviceChanged,
    Busy,
    NotAvailable,
    HostUnavailable,
    Unknown,
}

impl AudioStreamFailure {
    #[must_use]
    pub fn retry_delay(self, attempt: u32) -> Option<Duration> {
        match self {
            Self::DeviceChanged | Self::NotAvailable => Some(Duration::ZERO),
            Self::Busy => Some(Duration::from_millis(100 * u64::from(attempt.min(5) + 1))),
            Self::HostUnavailable => Some(Duration::from_secs(u64::from(attempt.min(4) + 1))),
            Self::Unknown if attempt < 3 => {
                Some(Duration::from_millis(250 * u64::from(attempt + 1)))
            }
            Self::Unknown => None,
        }
    }
}

#[derive(Default)]
pub struct FailureQueueMetrics {
    depth: AtomicUsize,
    dropped: AtomicU64,
}
impl FailureQueueMetrics {
    #[must_use]
    pub fn depth(&self) -> usize {
        self.depth.load(Ordering::Relaxed)
    }
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}
#[derive(Clone)]
pub struct FailureSender {
    inner: SyncSender<AudioStreamFailure>,
    metrics: Arc<FailureQueueMetrics>,
}
pub struct FailureReceiver {
    inner: Receiver<AudioStreamFailure>,
    metrics: Arc<FailureQueueMetrics>,
}
#[must_use]
pub fn failure_channel(
    capacity: usize,
) -> (FailureSender, FailureReceiver, Arc<FailureQueueMetrics>) {
    let (tx, rx) = mpsc::sync_channel(capacity.max(1));
    let metrics = Arc::new(FailureQueueMetrics::default());
    (
        FailureSender {
            inner: tx,
            metrics: metrics.clone(),
        },
        FailureReceiver {
            inner: rx,
            metrics: metrics.clone(),
        },
        metrics,
    )
}
impl FailureSender {
    /// Suitable for CPAL's error callback: bounded, non-blocking, allocation-free.
    #[must_use]
    pub fn notify(&self, failure: AudioStreamFailure) -> bool {
        match self.inner.try_send(failure) {
            Ok(()) => {
                self.metrics.depth.fetch_add(1, Ordering::Relaxed);
                true
            }
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.metrics.dropped.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }
}
impl FailureReceiver {
    #[must_use]
    pub fn try_recv(&self) -> Option<AudioStreamFailure> {
        match self.inner.try_recv() {
            Ok(value) => {
                self.metrics.depth.fetch_sub(1, Ordering::Relaxed);
                Some(value)
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }

    fn recv_timeout(&self, timeout: Duration) -> Option<AudioStreamFailure> {
        match self.inner.recv_timeout(timeout) {
            Ok(value) => {
                self.metrics.depth.fetch_sub(1, Ordering::Relaxed);
                Some(value)
            }
            Err(_) => None,
        }
    }
}

pub trait CaptureAdapter {
    type Stream;
    /// # Errors
    /// Returns a typed host/device enumeration failure.
    fn devices(&mut self) -> Result<Vec<String>, AudioStreamFailure>;
    /// # Errors
    /// Returns a typed config negotiation or stream construction failure.
    fn open(&mut self, id: Option<&str>) -> Result<Self::Stream, AudioStreamFailure>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryEvent {
    Recovering {
        failure: AudioStreamFailure,
        attempt: u32,
    },
    Recovered {
        fallback: bool,
    },
    Unavailable {
        failure: AudioStreamFailure,
    },
}

/// Dedicated owner for capture recovery. Stopping always joins the worker, so
/// neither the control thread nor its stream can outlive the service state.
pub struct RecoveryWorker {
    stop: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    join: Option<JoinHandle<()>>,
}

impl RecoveryWorker {
    /// Spawns the dedicated recovery control thread.
    ///
    /// # Panics
    ///
    /// Panics if the operating system refuses to create the worker thread.
    pub fn spawn<A, F>(
        adapter: A,
        selected: Option<String>,
        generation: u64,
        failures: FailureReceiver,
        publish: F,
    ) -> Self
    where
        A: CaptureAdapter + Send + 'static,
        A::Stream: Send + 'static,
        F: Fn(RecoveryEvent) + Send + 'static,
    {
        let stop = Arc::new(AtomicBool::new(false));
        let current_generation = Arc::new(AtomicU64::new(generation));
        let worker_stop = Arc::clone(&stop);
        let worker_generation = Arc::clone(&current_generation);
        let join = std::thread::Builder::new()
            .name("pw-audio-recovery".into())
            .spawn(move || {
                let mut controller = RecoveryController::new(adapter, selected, generation);
                while !worker_stop.load(Ordering::Acquire) {
                    let Some(mut failure) = failures.recv_timeout(Duration::from_millis(20)) else {
                        continue;
                    };
                    let event_generation = worker_generation.load(Ordering::Acquire);
                    let mut attempt = 0;
                    loop {
                        if worker_stop.load(Ordering::Acquire) {
                            break;
                        }
                        publish(RecoveryEvent::Recovering { failure, attempt });
                        match controller.recover(event_generation, failure) {
                            RecoveryOutcome::Recovered { fallback } => {
                                publish(RecoveryEvent::Recovered { fallback });
                                break;
                            }
                            RecoveryOutcome::Retry(next) => {
                                let Some(delay) = next.retry_delay(attempt) else {
                                    publish(RecoveryEvent::Unavailable { failure: next });
                                    break;
                                };
                                failure = next;
                                attempt += 1;
                                if delay > Duration::ZERO {
                                    std::thread::sleep(delay.min(Duration::from_millis(100)));
                                }
                            }
                            RecoveryOutcome::Stopped | RecoveryOutcome::Stale => break,
                        }
                    }
                }
                controller.stop();
            })
            .expect("failed to spawn audio recovery worker");
        Self {
            stop,
            generation: current_generation,
            join: Some(join),
        }
    }

    pub fn set_generation(&self, generation: u64) {
        self.generation.store(generation, Ordering::Release);
    }

    pub fn stop_and_join(mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for RecoveryWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum RecoveryOutcome {
    Recovered { fallback: bool },
    Retry(AudioStreamFailure),
    Stopped,
    Stale,
}
pub struct RecoveryController<A: CaptureAdapter> {
    adapter: A,
    selected: Option<String>,
    generation: u64,
    stopped: bool,
    stream: Option<A::Stream>,
}
impl<A: CaptureAdapter> RecoveryController<A> {
    pub fn new(adapter: A, selected: Option<String>, generation: u64) -> Self {
        Self {
            adapter,
            selected,
            generation,
            stopped: false,
            stream: None,
        }
    }
    pub fn stop(&mut self) {
        self.stopped = true;
        self.stream.take();
    }
    #[must_use]
    pub fn adapter(&self) -> &A {
        &self.adapter
    }
    pub fn recover(&mut self, generation: u64, failure: AudioStreamFailure) -> RecoveryOutcome {
        if generation != self.generation {
            return RecoveryOutcome::Stale;
        }
        if self.stopped {
            return RecoveryOutcome::Stopped;
        }
        self.stream.take();
        let fallback = if failure == AudioStreamFailure::NotAvailable {
            let devices = match self.adapter.devices() {
                Ok(v) => v,
                Err(e) => return RecoveryOutcome::Retry(e),
            };
            !self
                .selected
                .as_ref()
                .is_some_and(|id| devices.contains(id))
        } else {
            false
        };
        let id = if fallback {
            None
        } else {
            self.selected.as_deref()
        };
        match self.adapter.open(id) {
            Ok(stream) => {
                self.stream = Some(stream);
                RecoveryOutcome::Recovered { fallback }
            }
            Err(error) => RecoveryOutcome::Retry(error),
        }
    }
}
