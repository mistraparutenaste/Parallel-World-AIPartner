//! Speech detection concepts: turning VAD probabilities into speech
//! segments and filtering unreliable transcripts.

mod segmenter;
mod transcript_filter;

pub use segmenter::{SegmentEvent, SegmenterConfig, SpeechSegment, SpeechSegmenter};
pub use transcript_filter::{FilterConfig, RejectionReason, TranscriptCandidate, TranscriptFilter};
