//! Microphone capture on a dedicated audio thread.
//!
//! cpal streams are not `Send`, so a dedicated thread owns the
//! stream. The callback mixes to mono and pushes into a bounded
//! ring buffer; the consumer half is handed to the caller.

use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{DeviceId, SampleFormat};

use crate::mix::{push_mono_counting_drops, write_interleaved_as_mono};

/// Ring buffer capacity in samples (~2 s at 48 kHz mono).
const RING_CAPACITY: usize = 96_000;
/// Scratch buffer for one callback worth of mono samples.
const SCRATCH_CAPACITY: usize = 8_192;

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
}

impl CaptureSession {
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped_samples.load(Ordering::Relaxed)
    }
}

impl Drop for CaptureSession {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
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
    let device_id = device_id.map(str::to_owned);
    let (ready_tx, ready_rx) = mpsc::channel();
    let (stop_tx, stop_rx) = mpsc::channel::<()>();

    std::thread::Builder::new()
        .name("pw-audio-capture".into())
        .spawn(move || {
            let result = build_stream(device_id.as_deref());
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

    let (consumer, sample_rate, dropped) =
        ready_rx.recv().map_err(|_| CaptureError::ThreadGone)??;
    Ok(CaptureSession {
        consumer,
        sample_rate,
        dropped_samples: dropped,
        stop: Some(stop_tx),
    })
}

type SessionParts = (rtrb::Consumer<f32>, u32, Arc<AtomicU64>);

fn build_stream(device_id: Option<&str>) -> Result<(cpal::Stream, SessionParts), CaptureError> {
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
        )?,
        SampleFormat::U16 => build_typed_stream::<u16>(
            &device,
            config.into(),
            channels,
            producer,
            Arc::clone(&dropped),
        )?,
        _ => build_typed_stream::<f32>(
            &device,
            config.into(),
            channels,
            producer,
            Arc::clone(&dropped),
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
            |error| {
                tracing::warn!(%error, "audio input stream error");
            },
            None,
        )
        .map_err(CaptureError::BuildStream)
}
