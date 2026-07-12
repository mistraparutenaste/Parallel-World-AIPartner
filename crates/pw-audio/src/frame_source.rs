//! Adapter: capture session + resampler as a 16 kHz frame source.

use std::collections::VecDeque;

use pw_application::speech::{FrameRead, PortError, SpeechFrameSource};

use crate::capture::CaptureSession;
use crate::resample::MonoResampler;

/// Target sample rate expected by VAD and STT.
pub const STT_SAMPLE_RATE: u32 = 16_000;

/// Pure buffering stage: native-rate samples in, fixed-size 16 kHz
/// frames out. Kept separate from the capture session so it can be
/// unit-tested without hardware.
pub struct ResamplingFrameBuffer {
    resampler: MonoResampler,
    ready: VecDeque<f32>,
}

impl ResamplingFrameBuffer {
    /// # Errors
    ///
    /// Returns an error when the resampler cannot be constructed.
    pub fn new(input_rate: u32) -> Result<Self, crate::resample::ResampleError> {
        Ok(Self {
            resampler: MonoResampler::new(input_rate, STT_SAMPLE_RATE)?,
            ready: VecDeque::new(),
        })
    }

    /// Feeds native-rate mono samples.
    ///
    /// # Errors
    ///
    /// Returns an error when resampling fails.
    pub fn feed(&mut self, samples: &[f32]) -> Result<(), crate::resample::ResampleError> {
        let output = self.resampler.process(samples)?;
        self.ready.extend(output);
        Ok(())
    }

    /// Fills `frame` completely when enough output is buffered.
    pub fn read_frame(&mut self, frame: &mut [f32]) -> bool {
        if self.ready.len() < frame.len() {
            return false;
        }
        for slot in frame.iter_mut() {
            *slot = self.ready.pop_front().unwrap_or(0.0);
        }
        true
    }
}

/// Frame source over a live capture session.
pub struct CaptureFrameSource {
    session: CaptureSession,
    buffer: ResamplingFrameBuffer,
    drain_scratch: Vec<f32>,
}

impl CaptureFrameSource {
    /// # Errors
    ///
    /// Returns an error when the resampler cannot be constructed for
    /// the session's native rate.
    pub fn new(session: CaptureSession) -> Result<Self, crate::resample::ResampleError> {
        let buffer = ResamplingFrameBuffer::new(session.sample_rate)?;
        Ok(Self {
            session,
            buffer,
            drain_scratch: vec![0.0; 4096],
        })
    }

    /// Total samples dropped in the capture callback so far.
    #[must_use]
    pub fn dropped_samples(&self) -> u64 {
        self.session.dropped()
    }
}

impl SpeechFrameSource for CaptureFrameSource {
    fn read_frame(&mut self, frame: &mut [f32]) -> Result<FrameRead, PortError> {
        // Drain whatever the callback produced since the last call.
        loop {
            let mut drained = 0;
            while drained < self.drain_scratch.len() {
                match self.session.consumer.pop() {
                    Ok(sample) => {
                        self.drain_scratch[drained] = sample;
                        drained += 1;
                    }
                    Err(_) => break,
                }
            }
            if drained == 0 {
                break;
            }
            self.buffer
                .feed(&self.drain_scratch[..drained])
                .map_err(|error| PortError(error.to_string()))?;
            if drained < self.drain_scratch.len() {
                break;
            }
        }
        if self.buffer.read_frame(frame) {
            Ok(FrameRead::Frame)
        } else {
            Ok(FrameRead::Pending)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ResamplingFrameBuffer;

    #[test]
    fn produces_fixed_frames_from_48k_input() {
        let mut buffer = ResamplingFrameBuffer::new(48_000).unwrap();
        let mut frame = [0.0_f32; 512];
        // Not enough input yet.
        assert!(!buffer.read_frame(&mut frame));
        // 48k mono: 4800 samples -> ~1600 at 16k -> 3 full frames.
        buffer.feed(&vec![0.25_f32; 4_800]).unwrap();
        let mut frames = 0;
        while buffer.read_frame(&mut frame) {
            frames += 1;
        }
        assert!((2..=3).contains(&frames), "frames: {frames}");
    }

    #[test]
    fn passthrough_rates_produce_exact_frames() {
        let mut buffer = ResamplingFrameBuffer::new(16_000).unwrap();
        buffer.feed(&vec![0.5_f32; 1_024]).unwrap();
        let mut frame = [0.0_f32; 512];
        assert!(buffer.read_frame(&mut frame));
        assert!(buffer.read_frame(&mut frame));
        assert!(!buffer.read_frame(&mut frame));
        assert!((frame[0] - 0.5).abs() < 1e-6);
    }
}
