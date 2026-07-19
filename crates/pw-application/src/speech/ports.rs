//! Ports implemented by audio / VAD / STT adapters.

use pw_domain::speech::RejectionReason;

pub use crate::port_error::PortError;

/// Result of one frame read from the audio source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameRead {
    /// The frame buffer was completely filled.
    Frame,
    /// Not enough buffered audio yet; try again later.
    Pending,
    /// The source ended (device stopped or disconnected).
    Ended,
}

/// Pull-based source of 16 kHz mono frames.
pub trait SpeechFrameSource: Send {
    /// Fills `frame` completely or reports pending/ended.
    ///
    /// # Errors
    ///
    /// Returns [`PortError`] on unrecoverable source failure.
    fn read_frame(&mut self, frame: &mut [f32]) -> Result<FrameRead, PortError>;
}

/// Voice activity detection over one 16 kHz mono frame.
pub trait VoiceActivityDetector: Send {
    /// Speech probability in `0.0..=1.0` for the frame.
    ///
    /// # Errors
    ///
    /// Returns [`PortError`] when inference fails.
    fn probability(&mut self, frame: &[f32]) -> Result<f32, PortError>;
    /// Clears internal state between utterances.
    fn reset(&mut self);
}

/// Speech-to-text over one complete segment.
pub trait SpeechRecognizer: Send {
    /// Transcribes 16 kHz mono samples.
    ///
    /// # Errors
    ///
    /// Returns [`PortError`] when inference fails.
    fn transcribe(&mut self, samples: &[f32]) -> Result<String, PortError>;
}

/// Mutable references forward the port so callers can keep ownership
/// of a loaded model across pipeline runs.
impl<V: VoiceActivityDetector> VoiceActivityDetector for &mut V {
    fn probability(&mut self, frame: &[f32]) -> Result<f32, PortError> {
        (**self).probability(frame)
    }

    fn reset(&mut self) {
        (**self).reset();
    }
}

/// Mutable references forward the port so callers can keep ownership
/// of a loaded model across pipeline runs.
impl<R: SpeechRecognizer> SpeechRecognizer for &mut R {
    fn transcribe(&mut self, samples: &[f32]) -> Result<String, PortError> {
        (**self).transcribe(samples)
    }
}

/// Sink for pipeline outcomes (UI events and diagnostics).
pub trait SpeechEvents: Send {
    fn on_level(&self, rms: f32);
    fn on_speech_started(&self);
    fn on_transcript(&self, text: &str);
    fn on_rejected(&self, reason: RejectionReason);
    fn on_error(&self, message: &str);
}
