//! Microphone capture, bounded buffering and resampling.
//!
//! The cpal callback only converts sample formats, mixes to mono and
//! pushes into a bounded lock-free ring buffer; overflow is counted,
//! never blocked on. Everything else (resampling, VAD, STT) runs on
//! worker threads.

pub mod capture;
pub mod devices;
pub mod frame_source;
pub mod mix;
pub mod resample;
