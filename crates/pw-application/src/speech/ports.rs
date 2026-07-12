//! Ports implemented by audio / VAD / STT adapters.

use pw_domain::speech::RejectionReason;

/// Failure inside an adapter. Messages must not contain secrets.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct PortError(pub String);

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

/// Sink for pipeline outcomes (UI events and diagnostics).
pub trait SpeechEvents: Send {
    fn on_level(&self, rms: f32);
    fn on_speech_started(&self);
    fn on_transcript(&self, text: &str);
    fn on_rejected(&self, reason: RejectionReason);
    fn on_error(&self, message: &str);
}
