//! Ports implemented by the TTS adapter and the playback event sink.

use std::path::{Path, PathBuf};

use pw_domain::reply::TurnId;

use crate::PortError;

/// Synthesizes one sentence to a WAV file on disk (cache included);
/// the returned path is handed to the playback layer by identifier
/// only (基本設計 8章).
pub trait TtsSynthesizer: Send {
    /// # Errors
    ///
    /// Returns [`PortError`] when the engine is unreachable or
    /// synthesis fails.
    fn synthesize(&self, text: &str) -> Result<PathBuf, PortError>;
}

/// Sink for synthesized audio and playback control. Every payload
/// carries the turn id and a per-turn sequence number so the player
/// can keep strict order and drop stale audio.
///
/// Accounting rule: every sentence accepted by the queue is reported
/// exactly once, through [`Self::on_audio`] when it will be spoken and
/// [`Self::on_unspoken`] when it will not. A caller that withholds the
/// text until it is spoken can therefore always release it.
pub trait SpeechAudioSink: Send {
    fn on_audio(&self, turn: TurnId, seq: u32, wav_path: &Path, text: &str);
    /// Accepted text that will never reach the speaker: an unspeakable
    /// fragment, a failed synthesis, or the remainder of a failed or
    /// interrupted turn. The user is still owed this text.
    fn on_unspoken(&self, turn: TurnId, text: &str);
    /// Playback (and any queued audio) must halt immediately.
    fn on_stop(&self);
    /// Synthesis failed; the conversation continues text-only.
    fn on_error(&self, turn: TurnId, message: &str);
}
