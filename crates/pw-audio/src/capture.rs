//! Microphone capture on a dedicated audio thread.
//!
//! cpal streams are not `Send`, so a dedicated thread owns the
//! stream. The callback mixes to mono and pushes into a bounded
//! ring buffer; the consumer half is handed to the caller.

use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{DeviceId, SampleFormat};

use crate::mix::{push_mono_counting_drops, write_interleaved_as_mono};
use crate::recovery::{AudioStreamFailure, FailureSender};

/// Ring buffer capacity in samples (~2 s at 48 kHz mono).
const RING_CAPACITY: usize = 96_000;
/// Scratch buffer for one callback worth of mono samples.
const SCRATCH_CAPACITY: usize = 8_192;
/// Maximum time allowed for the platform audio backend to open and play a stream.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
/// Polling interval used so startup cancellation remains responsive.
const STARTUP_POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("audio device not found: {0}")]
    DeviceNotFound(String),
    #[error("invalid device id {0}: {1}")]
    InvalidDeviceId(String, cpal::Error),
    #[error("no default input device")]
    NoDefaultDevice,
    #[error("failed to query device config: {0}")]
    DeviceConfig(cpal::Error),
    #[error("failed to build input stream: {0}")]
    BuildStream(cpal::Error),
    #[error("failed to start input stream: {0}")]
    PlayStream(cpal::Error),
    #[error("audio thread terminated unexpectedly")]
    ThreadGone,
    #[error("audio capture startup was cancelled")]
    StartupCancelled,
    #[error("audio capture startup timed out")]
    StartupTimeout,
}

/// Running capture session. Dropping it stops the stream.
pub struct CaptureSession {
    /// Mono samples at [`CaptureSession::sample_rate`].
    pub consumer: rtrb::Consumer<f32>,
    /// Native sample rate of the capture stream.
    pub sample_rate: u32,
    /// Samples dropped because the ring buffer was full.
    pub dropped_samples: Arc<AtomicU64>,
    stop: Option<mpsc::Sender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl CaptureSession {
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped_samples.load(Ordering::Relaxed)
    }

    /// Stops capture and waits until the stream-owning thread has released the device.
    pub fn stop_and_join(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for CaptureSession {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

/// Starts capturing from the given device (default when `None`).
///
/// # Errors
///
/// Returns a [`CaptureError`] when the device cannot be resolved or
/// the stream cannot be built.
///
/// # Panics
///
/// Panics when the OS refuses to spawn the audio thread.
pub fn start_capture(device_id: Option<&str>) -> Result<CaptureSession, CaptureError> {
    start_capture_with_failures(device_id, None)
}

/// Starts capture and forwards CPAL stream failures to a bounded control channel.
///
/// The error callback performs only a non-blocking channel send. Recovery, logging,
/// device enumeration and stream rebuilding must happen on the owning control thread.
///
/// # Errors
///
/// Returns a [`CaptureError`] when the device cannot be resolved or the stream cannot be built.
///
/// # Panics
///
/// Panics when the OS refuses to spawn the dedicated capture thread.
pub fn start_capture_with_failures(
    device_id: Option<&str>,
    failures: Option<FailureSender>,
) -> Result<CaptureSession, CaptureError> {
    let cancel = AtomicBool::new(false);
    start_capture_with_failures_until_cancelled(device_id, failures, &cancel)
}

/// Starts capture while allowing the caller to cancel a platform backend that
/// does not return from stream initialization.
///
/// # Errors
///
/// Returns [`CaptureError::StartupCancelled`] when `cancel` is set, or
/// [`CaptureError::StartupTimeout`] when the backend does not respond in time.
///
/// # Panics
///
/// Panics when the OS refuses to spawn the dedicated capture thread.
pub fn start_capture_with_failures_until_cancelled(
    device_id: Option<&str>,
    failures: Option<FailureSender>,
    cancel: &AtomicBool,
) -> Result<CaptureSession, CaptureError> {
    let device_id = device_id.map(str::to_owned);
    let (ready_tx, ready_rx) = mpsc::channel();
    let (stop_tx, stop_rx) = mpsc::channel::<()>();

    let worker = std::thread::Builder::new()
        .name("pw-audio-capture".into())
        .spawn(move || {
            let result = build_stream(device_id.as_deref(), failures);
            match result {
                Ok((stream, session_parts)) => {
                    if let Err(error) = stream.play() {
                        let _ = ready_tx.send(Err(CaptureError::PlayStream(error)));
                        return;
                    }
                    let _ = ready_tx.send(Ok(session_parts));
                    // Own the stream until stop is requested or the
                    // controller is dropped.
                    let _ = stop_rx.recv();
                    drop(stream);
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                }
            }
        })
        .expect("failed to spawn audio capture thread");

    let (consumer, sample_rate, dropped) = wait_for_startup(&ready_rx, cancel, STARTUP_TIMEOUT)?;
    Ok(CaptureSession {
        consumer,
        sample_rate,
        dropped_samples: dropped,
        stop: Some(stop_tx),
        worker: Some(worker),
    })
}

fn wait_for_startup(
    ready_rx: &mpsc::Receiver<Result<SessionParts, CaptureError>>,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<SessionParts, CaptureError> {
    let deadline = Instant::now() + timeout;
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(CaptureError::StartupCancelled);
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(CaptureError::StartupTimeout);
        }
        let wait = STARTUP_POLL_INTERVAL.min(deadline.saturating_duration_since(now));
        match ready_rx.recv_timeout(wait) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(CaptureError::ThreadGone),
        }
    }
}

type SessionParts = (rtrb::Consumer<f32>, u32, Arc<AtomicU64>);

fn build_stream(
    device_id: Option<&str>,
    failures: Option<FailureSender>,
) -> Result<(cpal::Stream, SessionParts), CaptureError> {
    let host = cpal::default_host();
    let device = match device_id {
        Some(id) => {
            let parsed = DeviceId::from_str(id)
                .map_err(|error| CaptureError::InvalidDeviceId(id.to_owned(), error))?;
            host.device_by_id(&parsed)
                .ok_or_else(|| CaptureError::DeviceNotFound(id.to_owned()))?
        }
        None => host
            .default_input_device()
            .ok_or(CaptureError::NoDefaultDevice)?,
    };
    let config = device
        .default_input_config()
        .map_err(CaptureError::DeviceConfig)?;
    let sample_rate = config.sample_rate();
    let channels = usize::from(config.channels());

    let (producer, consumer) = rtrb::RingBuffer::<f32>::new(RING_CAPACITY);
    let dropped = Arc::new(AtomicU64::new(0));

    let stream = match config.sample_format() {
        SampleFormat::I16 => build_typed_stream::<i16>(
            &device,
            config.into(),
            channels,
            producer,
            Arc::clone(&dropped),
            failures.clone(),
        )?,
        SampleFormat::U16 => build_typed_stream::<u16>(
            &device,
            config.into(),
            channels,
            producer,
            Arc::clone(&dropped),
            failures.clone(),
        )?,
        _ => build_typed_stream::<f32>(
            &device,
            config.into(),
            channels,
            producer,
            Arc::clone(&dropped),
            failures,
        )?,
    };
    Ok((stream, (consumer, sample_rate, dropped)))
}

fn build_typed_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    mut producer: rtrb::Producer<f32>,
    dropped: Arc<AtomicU64>,
    failures: Option<FailureSender>,
) -> Result<cpal::Stream, CaptureError>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    // Preallocated outside the callback: the callback itself must
    // not allocate.
    let mut convert_scratch = vec![0.0_f32; SCRATCH_CAPACITY * 2];
    let mut mono_scratch = vec![0.0_f32; SCRATCH_CAPACITY];
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                let len = data.len().min(convert_scratch.len());
                for (target, sample) in convert_scratch[..len].iter_mut().zip(data) {
                    *target = cpal::Sample::from_sample(*sample);
                }
                let frames =
                    write_interleaved_as_mono(&convert_scratch[..len], channels, &mut mono_scratch);
                let lost = push_mono_counting_drops(&mut producer, &mono_scratch[..frames]);
                if lost > 0 {
                    dropped.fetch_add(lost as u64, Ordering::Relaxed);
                }
            },
            move |error| {
                if let Some(sender) = &failures {
                    let _ = sender.notify(classify_stream_error(&error));
                }
            },
            None,
        )
        .map_err(CaptureError::BuildStream)
}

fn classify_stream_error(error: &cpal::Error) -> AudioStreamFailure {
    match error.kind() {
        cpal::ErrorKind::DeviceChanged | cpal::ErrorKind::StreamInvalidated => {
            AudioStreamFailure::DeviceChanged
        }
        cpal::ErrorKind::DeviceBusy => AudioStreamFailure::Busy,
        cpal::ErrorKind::DeviceNotAvailable => AudioStreamFailure::NotAvailable,
        cpal::ErrorKind::HostUnavailable => AudioStreamFailure::HostUnavailable,
        _ => AudioStreamFailure::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::Duration;

    use super::{CaptureError, CaptureSession, wait_for_startup};

    #[test]
    fn startup_wait_honors_cancellation_when_audio_backend_does_not_respond() {
        let (_ready_tx, ready_rx) = std::sync::mpsc::channel();
        let cancel = AtomicBool::new(true);

        let result = wait_for_startup(&ready_rx, &cancel, Duration::from_secs(1));

        assert!(matches!(result, Err(CaptureError::StartupCancelled)));
    }

    #[test]
    fn startup_wait_times_out_when_audio_backend_does_not_respond() {
        let (_ready_tx, ready_rx) = std::sync::mpsc::channel();
        let cancel = AtomicBool::new(false);

        let result = wait_for_startup(&ready_rx, &cancel, Duration::ZERO);

        assert!(matches!(result, Err(CaptureError::StartupTimeout)));
    }

    #[test]
    fn dropping_session_joins_stream_owner() {
        let (_producer, consumer) = rtrb::RingBuffer::<f32>::new(1);
        let (stop_tx, stop_rx) = std::sync::mpsc::channel();
        let exited = Arc::new(AtomicBool::new(false));
        let worker_exited = Arc::clone(&exited);
        let worker = std::thread::spawn(move || {
            let _ = stop_rx.recv();
            worker_exited.store(true, Ordering::Release);
        });
        let session = CaptureSession {
            consumer,
            sample_rate: 48_000,
            dropped_samples: Arc::new(AtomicU64::new(0)),
            stop: Some(stop_tx),
            worker: Some(worker),
        };

        drop(session);

        assert!(exited.load(Ordering::Acquire));
    }
}
